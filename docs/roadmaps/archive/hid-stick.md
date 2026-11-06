# 外设模式：外设驱动硬件注入 — 改造路线图 / Change Roadmap

Status: archived（核心通道已发布；Raw Input 同键支持与剩余真机验收保留为后续项）
Goal: 助手多一档「外设模式」：按键事件经 USB 串口发给外设驱动，由外设驱动作为真实 USB 键鼠敲出；即插即用、不要管理员、不用重启，目标键等于触发键的切换规则可用。

---

## 1. 背景 / Background

现有三档注入通道都在电脑上生成事件：SendInput 带 `SIM_MARKER`，钩子能认出自己；Interception 与 DD 在驱动层注入，自注入只能靠 `PENDING_INJECTIONS` 的时间窗队列过滤，所以 DD 系列禁止切换规则的目标键等于启动键（`InputMode::requires_distinct_target_for_toggle`），横版布局也因此与它互斥。

外设驱动是 xwink-body 仓做的 ESP32-S3 小板（那边叫「HID 棒」，[xwink-body 具身宿主路线图 D29 到 D32]），USB 复合设备：HID 键盘、HID 鼠标、CDC 串口。它同时是那台机器人的「手」，两个用途共用一个 HID 层。对助手来说它的本质区别是**一台独立的物理设备**：Windows Raw Input 的 `hDevice` 能精确说出每个事件来自哪台键鼠，而 Interception 注入的事件在 Raw Input 里就是用户的真键盘。

| 能力 | 现状 |
| --- | --- |
| 统一分发入口 `win_input::dispatch(KeyId, is_up)` | 已有，按 `(mode, KeyId)` 分发到后端，加一档不动引擎 |
| 后端接口 `send_key / send_mouse / send_wheel` | 已有，三个后端同形 |
| 钩子线程与消息循环 | 已有，`burst-engine` 的 `WH_KEYBOARD_LL` 与 `WH_MOUSE_LL` 共用一条线程 |
| Raw Input 通道 | 缺失 |
| 串口通信 | 缺失，工作区没有串口 crate |
| 外设驱动指令协议 | 由固件定，以 xwink-body 的 `skills/xwink-body/references/engineering/firmware.md` 的 hand 一节为准 |

不变量：全局总开关关着时不发键；引擎线程异常、切换模式、外设驱动拔出都不能留下按下未释放的键；`win-*` 不接受 `AppHandle`；配置装载仍只经 `activate_profile_file`。

## 2. 决策 / Decisions

- **D1:** 第四档 `InputMode::HidStick`（序列化名 `hidstick`），后端在 `packages/win-input/src/hidstick.rs` — 理由：`dispatch` 已是统一入口，加一档是最小侵入；引擎与规则层零改动。
- **D2:** 外设模式下触发检测走 Raw Input，按设备句柄过滤，外设驱动发回的事件不进规则判定；低级钩子仍负责全局热键与需要拦截的场景 — 理由：这是外设模式能做「目标等于触发」而 DD 不能的唯一原因。判定只吃 Raw Input 一条流，不依赖 `WM_INPUT` 与钩子回调谁先到，两者都由 RIT 派发、顺序没有文档保证。
- **D3:** 通道用 CDC 串口行协议，每个注入事件一条指令，连发调度仍在电脑 — 理由：引擎不动；usbser 写入约 1 ms 加 HID 中断端点 1 ms 轮询，总时延两三毫秒，远小于 10 ms 的有效间隔下限。助手挨个 USB 串口握手认口（跳过 CH34x、CP210x 等转串口芯片，它们常接开发板复位脚），不让用户选；不认 VID/PID，PID 是固件自定的、换板子就变。
- **D4:** 断开即松键两端都做：外设驱动 DTR 掉或心跳超时松开全部；助手切出外设模式或外设驱动拔出时清状态、回退 SendInput 并 warn 一次 — 理由：卡键是这类外设最常见的事故。
- **D5:** 不伪装 VID/PID，不承诺反检测；README 末尾的风险提示对外设模式同样成立 — 理由：冒用别家号是另一件事；能被查的是节奏不是设备，硬件 HID 与外设厂商的宏键盘同类。
- **D6:** 外设模式不与横版布局互斥，`requires_distinct_target_for_toggle` 对它返回 `false` — 理由：互斥的根源是 DD 下重合态切换连发是空集，D2 解决了这个前提。
- **D7:** 不在外设模式时外设驱动连上（插上，或启动时本就插着）就弹窗问要不要切换，同意后走与手动选模式同一条 `selectInputMode`；上次就是外设模式则启动时直接恢复、不问；横版下只提示不弹确认 — 理由：插着外设驱动就是想用它（用户实测要求启动时也问）。代价是特意选了别的模式又一直插着的人每次启动都被问，需要时再加「记住选择」。
- **D8:** 后端只报连接态翻转：插拔监视（后台每 1.5 s 枚举乐鑫的 USB 串口，新口握一次手；手动选外设模式与诊断才握全部非转串口芯片的口）与链路断开（心跳 / 下发失败）汇成 `hid-stick-connected` / `hid-stick-disconnected` 两个事件，不替前端刷新状态、不管谁在听 — 理由：后端报事实、前端按用户操作路径补齐，与恢复配置输入模式同一思路。
- **D9:** 逐条等固件回复（`OK` / `ERR`），心跳只在没按着键时发 — 理由：往返实测不到 1 ms，换来拔出与六键已满当场可见；按着键还心跳会替卡死的引擎续命，废掉固件「按着 1 s 无指令就松开」那道保险。
- **P1（按推荐默认先行，可改）：** 外设模式不算亲友专属功能（[license](../license.md) 只有水印署名一项权益），外设驱动本身就是门槛，助手侧保持免费。

## 3. 问题清单 / Findings

| # | 问题 | 严重度 | 位置 | → 任务卡 |
| --- | --- | --- | --- | --- |
| I1 | `PENDING_INJECTIONS` 时间窗对目标等于触发的切换规则不可靠，会自停或停不掉 | P1 | `packages/win-input/src/lib.rs:239` | T2 |
| I2 | 外设驱动拔出或串口写失败后若继续下发，规则静默失效且可能留下按下未释放 | P1 | `packages/win-input/src/hidstick.rs`（新） | T1 |
| I3 | 设置弹窗与 `requires_admin` 都假设只有三档 | P2 | `apps/main/src/windows/panel/dialogs/SettingsDialog.tsx`、`packages/win-input/src/lib.rs:243` | T3 |
| I4 | 说明书只讲三档通道 | P2 | `skills/flair-bloom/references/manual/game-mode.md` | T3 |

## 4. 任务卡 / Task cards

### T1 — 外设模式后端 · 进度: 完成（2026-10-08）
- **Fixes:** I2
- **Change:** `win-input` 加 `hidstick.rs`：挨个 USB 串口握手找口并打开；`send_key / send_mouse / send_wheel` 翻译成外设驱动的行协议，VK 到 HID usage 一张表，鼠标五键与滚轮原生；后台心跳；写失败即标记离线、清挂起注入、回退 SendInput 并 warn 一次。`InputMode` 加 `HidStick`（`from_str` / `as_str` / `requires_admin` 返回 `false`），`init_backend` 加分支。工作区引入一个串口 crate。
- **Acceptance:** 外设模式下长按连发与切换连发能跑；拔掉外设驱动后一秒内引擎无卡键、日志一条 warn、模式回退。
- **Test:** 单测覆盖 usage 表（字母、数字、功能键、修饰键、小键盘）、协议编码、离线回退路径；真机：记事本里长按 Q 连发。
- **Rollback:** 设置切回任一现有模式即可，不动配置文件 schema。
- **Depends on:** xwink-body F5a（固件与协议定稿）。
- **Done:** `win-input/src/hidstick.rs` + `InputMode::HidStick`；真机单测 `cargo test -p win-input hidstick -- --ignored`（敲 F13、空闲过心跳、媒体键回退）在固件 `b5a8bacc` 上通过。拔外设驱动回退与记事本长按 Q 留给 T4。

### T2 — Raw Input 检测通道 · 进度: 未开始
- **Fixes:** I1
- **Change:** `burst-engine` 在钩子那条消息循环线程建 message-only 窗口，注册键盘与鼠标的 `RIDEV_INPUTSINK`，事件带设备路径；外设模式下 `on_key_press / release` 的输入源改为 Raw Input 并过滤外设驱动设备，钩子只保留热键与需要拦截的规则；`requires_distinct_target_for_toggle` 对 `HidStick` 返回 `false`，布局互斥判断不含它。非 Windows 保持空实现。
- **Acceptance:** 目标等于触发的切换规则在外设模式下起停正常；互斥组插队栈不被外设驱动的回灌误触发。
- **Test:** 单测：设备过滤与事件到规则的映射用假事件流；真机按 [testing](../../../skills/flair-bloom/references/engineering/testing.md) 的冒烟清单过三类规则加互斥组。
- **Rollback:** 只在 `HidStick` 模式启用 Raw Input 路径，其余模式行为不变。
- **Depends on:** T1。
- **逼出 / Why:** 引擎第一次拥有「事件来自哪台设备」这个信息，DD 的时间窗约束以后也可能靠它放宽。

### T3 — 设置、状态与说明书 · 进度: 完成（2026-10-08）
- **Fixes:** I3、I4
- **Change:** 设置弹窗加「外设模式」档位，状态区显示外设驱动在线与否；`manual/` 加 `hid-stick.md` 并按 `pnpm skills:check` 补对应教程条目；README 通道说明与风险提示各补一句。
- **Acceptance:** 用户不看源码能按说明书把外设驱动用起来；`pnpm skills:check` 通过。
- **Test:** `pnpm skills:check`、`pnpm test:ui`。
- **Depends on:** T1。
- **Done:** 设置弹窗与模式菜单加「外设模式」，状态带 `hid_stick_present`，插入弹窗（D7）与断开提示；说明书没有单开教程，并进 `manual/game-mode.md` 常见坑与 `layouts.md`——外设模式只给有外设驱动的人，单开一组教程不值。

### T4 — 真机冒烟 · 进度: 进行中（2026-10-08 起）
- **Change:** 在测试配置里按说明书走一遍：三类规则、互斥组、插队、拔外设驱动、切模式；别的模式下插外设驱动弹窗、同意后切换；外设模式下拔外设驱动提示并回退、插回再问；启动时按上次模式恢复。
- **Acceptance:** 冒烟清单全绿。
- **Depends on:** T2、T3。
- **已过：** `cargo run -p burst-engine --example hidstick_smoke` 往记事本逐键打字（按住 10 ms、间隔 10 ms）一字不差，引擎经外设驱动长按连发 Q 1 s 打出约 20 个；诊断修复的串口扫描与设备槽检查在本机读数正确。**未过：** 应用内插拔弹窗、拔棒回退、互斥组与插队。

## 5. 依赖与顺序 / Dependencies & order

```
xwink-body F5a ─> T1 ─> T2 ─> T4
                  └──> T3 ─┘
```

- **执行顺序 / Order:** 固件 F5a → T1 → T3 → T2 → T4。
- **最小可交付里程碑 / Minimum valuable milestone:** T1 加 T3。外设模式能连发、即插即用；切换规则暂沿用 DD 的约束，T2 才解锁目标等于触发。

## 6. 工期 / Effort

口径：单人加 AI 协作，单位「会话」，与 [onboarding-tour](onboarding-tour.md) 同口径。校准来源：那份路线图 6.5 会话按预估完成。

| 任务卡 | 预估 | 实际 | 偏差说明 |
| --- | --- | --- | --- |
| T1 | 1 会话 | 0.6 会话 | 固件协议已在真机验过，握手一次通过 |
| T2 | 1.5 会话 | | |
| T3 | 0.5 会话 | 0.4 会话 | 中途加了插入弹窗（D7）与事件收口（D8） |
| T4 | 0.5 会话 | | |
| **合计** | **3.5 会话** | | |

拉长日历但不占净工时：固件 F5a 先行；送人的外设驱动换小板（S3 SuperMini 或 S3 Zero）要再采购。

## 7. 风险与回滚 / Risks & rollback

- **R1 Raw Input 与钩子看到同一事件的先后与重复**：判定只取 Raw Input 一条流，钩子路径不再对外设驱动回灌做判断，就不存在对账。
- **R2 CDC 在部分机箱前面板口握手失败**（实测过描述符请求失败）：说明书写明插主板后置口；助手枚举不到外设驱动时提示这一点。
- **R3 反作弊按行为检测**：与现有三档同一风险，D5 不做承诺。
- **R4 Interception 设备槽耗尽吞掉外设驱动的键盘**（2026-10-08 本机实测：开机 12 天、装卸 DD-HID 留下 10 个虚拟键盘，外设驱动的键盘分不到槽，鼠标在 18 号槽照常）：诊断修复报「外设驱动的输入被吞」，只建议重启。**不做不重启修复**：同日实测「一起移除全部键盘再重扫」，`keyboard.sys` 并未卸载（仍 RUNNING），重新加回的键盘——包括用户自己的实体键盘——全都分不到槽而失灵，拔插也救不回，只能重启。

## 8. 范围外 / Out of scope · follow-ups

- 键盘经外设驱动中转的中间人形态（外设驱动加 MAX3421E 当 USB 主机，规则全跑在片上）。
- 绝对坐标鼠标（HID 数位板描述符）。
- 助手经 CDC 给外设驱动升级固件。
- v0.5.0 发版前审查留下的低优先级项（不卡键、不死锁、不拖慢启动，下个版本处理）：
  - 下发失败引起的一时断链后重握手，可能撞上释放线程还没关口，口被记进 `seen` 后要拔插一次才重新认（`bootstrap/input.rs` 监视线程；可改为 `forget` 后头一两轮握手失败不记 `seen`，或释放完再报断开）。
  - 非乐鑫芯片的外设驱动手动连上后切走模式再拔掉，连接态停在「已插入」，只影响显示。
  - 外设模式下卸载 / 修复驱动不再顺带释放同一次运行里残留的 Interception / DD 后端（`init_backend(HidStick)` 不释放它们）；可在外设模式分支只释放这两个。
  - 外设模式打开失败的 300 ms 重试跑在主线程，上次是外设模式而这次没插时启动多卡一下、非转串口芯片的口被拉两次 DTR。
  - 插拔监视看到口消失时 `hidstick_link_down` 不核对口名，同时插两个乐鑫外设驱动时拔掉没在用的那个也会断开在用的。
