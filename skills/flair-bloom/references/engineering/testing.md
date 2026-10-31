# 测试策略与已落地的测试层

按键助手的测试难点在于：连发依赖低级 hook（`WH_KEYBOARD_LL`/`WH_MOUSE_LL`）与驱动注入（SendInput / DD / Interception），这些跑在 OS 输入栈上、需要交互式桌面会话，传统 headless CI 跑不了。业界同类工具（[kanata](https://github.com/jtroo/kanata) 的 `simulated_input`、[PowerToys Keyboard Manager](https://github.com/microsoft/PowerToys) 的 `Input` 抽象）的共识是**分层**：能在不碰 OS 的前提下确定性验证的逻辑，全部下沉到模拟测试；真正碰 OS/驱动的那一层才手动或半自动验。

## 三层

| 层                        | 测什么                                                 | 怎么测                                   | 跑在哪                                    | 状态                                        |
| ------------------------- | ------------------------------------------------------ | ---------------------------------------- | ----------------------------------------- | ------------------------------------------- |
| **L1 引擎逻辑（确定性）** | 连发时序、Hold/Toggle/分组/停止/热键/共享目标/throttle | 虚拟时钟 + 录制 dispatcher，纯 Rust 单测 | CI（`ci.yml`，windows-latest）            | ✅ 已落地                                   |
| **L1.5 引擎管线**         | 按键 → 引擎 → 下发给调度器的命令 + generation          | 命令录制替身                             | CI（同上）                                | ✅ 已落地                                   |
| **L2 真 OS 冒烟**         | SendInput → LL hook → SIM_MARKER 往返                  | 自装 hook 捕获并吞掉自注入               | 本机 / 自建 Windows runner（`#[ignore]`） | ✅ SendInput 已落地；DD/Interception 待真机 |
| **L3 驱动/反作弊**        | DD/Interception 真驱动是否打进游戏                     | 手动                                     | 真机                                      | 不可自动化                                  |

**核心认知**：当前发版前手动验的边界，80–90% 是 L1/L1.5 引擎逻辑——这些已自动化、已进 CI 门禁。真正不可自动化的只剩 L3「注入有没有真打进游戏」，竞品也是手动验。

## CI 门禁

`.github/workflows/ci.yml`（push:main / PR 触发，windows-latest）：`cargo fmt --check` + `cargo clippy -D warnings` + `cargo test`（排除 Tauri app `flair-bloom`，其构建依赖前端 dist；逻辑都在各 crate）。L1/L1.5/引擎/边界用例每次推送自动跑。L2 冒烟为 `#[ignore]` 不在 CI 跑（需交互式会话）。共享 crate 覆盖率另由 `coverage.yml` 把守 ≥85%。

前端没有测试框架，界面里可抽成纯函数的几何 / 判定逻辑放 `scripts/*.test.ts`，用 Node 内置 test runner 加类型剥离直接跑：`pnpm test:ui`（Node ≥ 22.18，更早的 22.x 类型剥离要加 `--experimental-strip-types` 才生效）。目前覆盖横版走线（`panel/hkbWires.ts`：边集合、参与键、扇出偏移、出线侧选择与嵌套不交叉、切角、端点缺失）、Markdown 内联切分（`components/markdown-parse.ts`）与主题对比度（`scripts/theme-contrast.test.ts`：全部预设 × 亮暗下文字四档、语义色文字、主色文字、实底前景三态都过 4.5:1，淡染 token 不超上限，`MODE_SURFACES` 与 `theme.css` 一致）。不进 `ci.yml`——那里只装 Rust 工具链；pre-commit 在暂存了 TS 文件时跑，`release.yml` 已装 Node，发版前再跑一遍兜底。Markdown 那条「不得用后行断言」是 macOS 12 面板白屏的护栏，不能只靠人记得本地跑。

## L1 已落地：调度器模拟 harness

代码：`packages/burst-engine/src/scheduler/sim_tests.rs`。

原理：连发时序逻辑全在 `SchedulerWorker::process_due(now)` 里，且它**把当前时刻作为入参**。喂合成的 `now`（虚拟时钟，逐毫秒推进）+ 一个只记录事件的 `RecordingDispatcher`（替代真实注入），即可对注入事件序列做 golden 断言——零 OS、完全确定性。

脚本 DSL（空白分隔）：

| token        | 含义                |
| ------------ | ------------------- |
| `start:<id>` | 启动规则            |
| `stop:<id>`  | 停止规则            |
| `stopall`    | 停止全部            |
| `t:<ms>`     | 推进虚拟时间 N 毫秒 |

输出：`<毫秒>:dn:<键>` / `<毫秒>:up:<键>`，按 (时刻, 抬起, 键) 稳定排序后空格连接。键渲染：键盘 `K<十六进制VK>`，鼠标 `M<按钮名>`。

**加一个场景测试 = 一行脚本 + 一个期望串**：

```rust
#[test]
fn hold_mode_separates_down_and_up_by_hold_duration() {
    let out = simulate(vec![hold("r", E, 10)], "start:r t:25");
    assert_eq!(out, "1:dn:K45 4:up:K45 11:dn:K45 14:up:K45 21:dn:K45 24:up:K45");
}
```

### L1 当前覆盖

- 调度器层（`scheduler/sim_tests.rs`）：点按档(1ms)同拍 down+up、Hold 档 down/up 按时长分离、节拍稳定、interval 边界(2ms 仍有 1ms 按下)、停止释放按住的目标键、间隔期停止不补多余 up、共享目标只发一次 down、共享目标 stopall 只释放一次、多目标 stopall 全释放、停止其一不影响其余、停止后干净重启、鼠标目标键。
- 引擎状态机层（`burst-engine/src/lib.rs` `#[cfg(test)] mod tests`）：Hold 首按启动/松开停止、Toggle 切换、Toggle 独立停止键、分组互斥顶替 + 顶替后复位重启、热键长按去重、鼠标键去重、专用停止键先于去重 100% 触发、停止复位物理账本、全局禁用拦截启动。
- 引擎管线层（`burst-engine/src/pipeline_tests.rs`）：注入命令录制替身，断言「按键 → 引擎 → 实际下发给调度器的命令 + generation」——Hold 启停、Toggle 同键开关、分组顶替先 stop 旧再 start 新、关全局开关 / 专用停止键下发 stop_all 且 generation 递增。抓 `active_ids` 测不到的引擎↔调度契约 bug。替身与规则构造器在 `test_support.rs` 共用。
- 组内插队层（`burst-engine/src/preempt_tests.rs`）：对应「长按插队，切换让位；松手恢复」行为表——插队先停后启、松手恢复、多长按嵌套逐个松开、栈中间先松只出栈、暂停期间停止键 / 同组替换（被暂停的不发 stop、新规则保持暂停）、无分组与跨组互不影响、滚轮长按不插队、全局关闭 / 切后端 / 退出清栈、两线程并发松长按与按停止键时调度器命令顺序与状态一致，以及 `get_rule_states` 分区。
- 纯逻辑层（共享 crate，覆盖率门槛 ≥85%）：`KeyId`、profile 校验、schema 迁移、AES/Ed25519、注入回灌过滤队列、throttle 升降数学。

运行：`cargo test -p burst-engine`（含 sim_tests）。

### 调度器压力测试（干跑，不发真实输入）

`packages/burst-engine/src/bin/scheduler_stress.rs`：把单调度线程、规则计时、StopAll 的 ACK 路径、generation 丢弃路径、同目标键合并跑到极限，用来看 64 条规则 10ms 间隔下调度延迟与线程数是否失控。

```sh
cargo run -p burst-engine --bin scheduler_stress --release -- --rules 64 --interval-ms 10 --duration-ms 10000   # 单场景
cargo run -p burst-engine --bin scheduler_stress --release -- --matrix --duration-ms 10000                       # 全矩阵
cargo run -p burst-engine --bin scheduler_stress --release -- --rules 64 --interval-ms 10 --duration-ms 10000 --same-target   # 同目标键合并
```

每行输出一条 JSON：`rules` / `interval_ms`、`scheduler_threads`（应为 1）、`hp_degraded`（高精度定时器退化为普通等待时为 true）、`sent_events` / `failed_events`、`injection_rate_per_sec`、`delay_p50_us` … `delay_max_us`（调度截止时刻的延迟）、`stop_ack_us`（StopAll 响应时间）、`process_cpu_ms`（仅 Windows）。

## L2 已落地：真 OS 往返冒烟

代码：`packages/burst-engine/src/smoke_tests.rs`（`#[cfg(all(test, windows))]` + `#[ignore]`）。

自装一个 `WH_KEYBOARD_LL` hook，对带 `SIM_MARKER` 的自注入事件**捕获后直接吞掉**（返回 1，不下传）——既验证 `win_input` SendInput 注入 → LL hook → 还原出正确 vk/抬起/`SIM_MARKER` 整条真实链路，又**零副作用**（注入键不泄漏到前台窗口，物理键照常放行）。需交互式桌面会话，故 `#[ignore]`：

```sh
cargo test -p burst-engine -- --ignored
```

**仍待真机/真驱动**：DD-HID / DDSimple / Interception 后端的往返需装对应驱动（且 DD 系列需管理员），同样的 hook + 吞自注入框架可扩展过去，但 GitHub 托管 runner 装不了驱动，需自建带驱动的 Windows runner，或在发版前本机手动跑。

> L1.5（引擎管线确定性）已落地：`BurstEngine` 的调度器抽象成 `scheduler::Scheduler` trait，测试经 `BurstEngine::new_with_scheduler` 注入命令录制替身，断言引擎下发的命令序列。见 `pipeline_tests.rs`。

**普通单测绝不真注入**：引擎的真注入只有 `scheduler.rs` 的 `inject` 一个出口，测试构建里它当作发送成功、什么也不发。原因：用 `BurstEngine::new()` 的单测会把规则真跑起来，目标键按 10 ms 连发进当前有焦点的窗口，跑 `cargo test` / `pnpm coverage` 时正在打字的输入框首当其冲。要真注入只走上面 `#[ignore]` 的 L2 冒烟与 `win-input` 的外设驱动真机测试（只敲 F13），二者都直接调 `win_input`。外设驱动往前台窗口真打字用 `cargo run -p burst-engine --example hidstick_smoke`（example 不是测试构建，会真注入）：先把光标放进记事本、别碰键鼠，结果以截图为准——新版记事本的 UI Automation 读回不全。打字节奏要对齐引擎（按住 10 ms、间隔 10 ms），按下松开紧挨着发约 2 ms 一键时记事本会丢字、回车错位，那是打得比任何键盘都快，不是通道问题。验证测试期间有没有漏出输入：前后读 `GetLastInputInfo`，SendInput 与真硬件事件都会刷新它。

## L3：发版前真机手动清单

显示缩放回归：系统文字大小 100%／110%／150%，显示器缩放 100%／150%，分别验证横版、竖版、浮窗和应用缩放；包含启动时已放大、运行中更改、最大化后更改、复位与切换布局。改变文字大小不应改变应用界面尺寸，键盘和浮窗展开按钮不能越界；跨屏 DPI 和应用拖窗放大仍须生效。脚本 `scripts/diagnose-environment.ps1` 只采外部环境；“关于 → 复制环境信息”补充当前页面视口、DPR 与实际 WebView 控制器缩放，内容边界仍需通过测试 WebView 采样。

环境诊断回归：普通用户运行统一脚本及两个旧名称入口，确认每份文本报告同时含显示与驱动段，权限错误不阻断其余采集；检查设置与文字大小未改变。“关于”每次打开、刷新与复制应重采集，双屏各自 DPI、客户端物理尺寸与 CSS 视口应与独立测量一致；模拟采集失败时，其余信息仍可复制且带错误。应用内“导出诊断报告”使用同一脚本，附加实际 WebView 缩放，结果写桌面；确认所有驱动安装、卸载及修复的重启提示明确区分 Windows“重启”和快速启动下的关机开机。

压力测试与冒烟都不发真实输入，发版前在 Windows 真机用 release 包按可用后端（SendInput / DDSimple / Interception）各验一遍：

- 面板聚焦、`trigger == target` 的 Toggle：注入事件必须被中继过滤吞掉，不能反复翻转 Toggle。
- 物理目标键按住时另一条触发键生效：调度器对该目标键跳过新的模拟 `down`，直到物理键松开。
- 模拟目标键已按下、再物理按下同键、然后 Stop / StopAll：模拟 `up` 仍必须发出。
- 64 条规则 10ms：进程线程数不随规则数增长；同目标键的规则只发一次合并的按住。
- 负载下 StopAll：接受后不再有新的目标 `down`，引擎持有的目标键全部释放。

## 已知不可自动化的边界

- **DD 同键 Hold（按住 Q 连发 Q）偶发自停**：DD 驱动清零 `ExtraInformation`，`SIM_MARKER` 无法幸存，hook 只能靠 `PENDING_INJECTIONS` 时间窗口尽力过滤，对同键无法可靠区分自注入与物理松手。这是 DD 驱动固有缺陷（竞品同样无解），L1 测不出，L2 sink 能观测到泄漏但做不成确定性断言。彻底消除需 Raw Input 按设备来源区分虚拟/物理设备。
