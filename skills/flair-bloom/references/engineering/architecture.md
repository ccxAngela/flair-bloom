# 架构

气质花（FlairBloom）是面向游戏辅助的按键助手：Tauri v2 单进程，Rust 后端跑全局键鼠钩子与注入，React 前端做面板与浮窗。核心功能免费，亲友专属功能通过 Ed25519 离线兑换码激活。

## 应用模式

| 模式                       | 现状                                                                                                                                                  |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| 面板（`panel.html`）       | 全功能配置界面。一切最小化路径都收进浮窗——标题栏按钮、面板显隐热键，以及系统级最小化（Win+D / 任务栏 / Aero Shake，由 `on_window_event` 的 Resized + `is_minimized` 接管，先 `unminimize` 再进浮窗模式）；关闭按钮与 Alt+F4 都走 `handleClose`，按 `closeBehavior` 偏好收进浮窗或退出，未设偏好时每次询问（默认） |
| 浮窗（`panel-float.html`） | 常驻置顶胶囊，显示激活规则、全局开关、展开主面板。Alt+F4 关浮窗 = 退出应用（`CloseRequested` 被 `prevent_close` 后转 `shutdown_and_exit`）：浮窗只有一行胶囊大小，放不下确认对话框，而放任它销毁会让收起状态的应用只剩托盘，且窗口销毁后无法再 `show` 回来 |
| 托盘                       | 面板隐藏后进程常驻，连发继续有效。托盘菜单：全局开关、切换配置（动态菜单项）、打开面板、退出；托盘双击与再次启动应用都唤回面板；启用 / 禁用态切换图标。托盘是辅助入口而不是唯一入口：浮窗不占任务栏位，若面板也缩进任务栏，应用就只剩一个会被折叠起来的托盘图标 |
| 桌宠（`pet.html`）         | 未实现，设计见 `docs/roadmaps/pet-mode.md`（draft）                                                                                                   |

## 进程模型

```
FlairBloom.exe（单一 Tauri 进程）
  Rust 后端：连发引擎 / 系统托盘 / 更新检查 / 驱动诊断 / 许可证校验
      │ Tauri events + invoke
      ├── 面板 WebView（panel.html）
      └── 浮窗 WebView（panel-float.html）
           │ HTTPS
   GitHub Releases ← gh-proxy.com 加速（直连兜底）
```

**两态不变量**：运行时面板与浮窗恰有一个可见，不存在「两个都藏起来只剩托盘」的中间态——托盘会被用户折叠，那时应用等于失踪。守住它的是三处：系统级最小化被 `on_window_event` 接管进浮窗模式、浮窗的关闭请求转为退出、`enter_float_mode` 在浮窗缺失时保留面板。

**退出只有一个出口**：`lib.rs` 的 `shutdown_and_exit`（先 `engine.shutdown()` 再 `app.exit(0)`），托盘「退出」、`exit_app` 命令（面板关闭选「直接退出」、协议对话框「不同意并退出」）、浮窗关闭请求都走它。不走 `window.destroy()`：销毁面板时浮窗窗口仍在窗口表里，进程不会退出，引擎也不会停，结果是窗口全没了但连发还在后台跑。`RunEvent::Exit` 里再 `shutdown()` 一次，兜住不经此函数的退出路径。

**单进程多窗口**：面板与浮窗是同一进程的独立 WebView，通过 `app.emit_all()` 事件通信（`float-active` / `global-enabled-changed` / `theme-changed` / `app-status-changed` / `update-*`），无 Named Pipe。窗口显隐统一走 `lib.rs` 的 `enter_panel_mode`（显示面板、隐藏浮窗）/ `enter_float_mode`（先显示浮窗再隐藏面板，浮窗缺失则保留面板）。激活态规则由前端轮询 `get_rule_states`（`panel/useRuleStates.ts`，面板与浮窗共用；运行 / 暂停分区取自同一快照），活跃集合在前端派生为两者并集。

**全局开关的切换都广播**：面板 / 浮窗点开关走 `set_global_enabled` 命令，热键走引擎回调，托盘走菜单事件，三条路都刷新托盘（菜单与图标）并广播 `global-enabled-changed`。语音播报只在面板里、只看 `globalEnabled` 状态翻转，且跳过启动加载期。因此「启动后自动开全局」不在后端 `setup` 里直接开——那样面板首帧读到的已是「开」，播报无从触发：后端 `should_auto_enable_on_start` 只做判定（设置、协议已同意、未开机自启三道闸），面板等配置、输入模式恢复（含提权确认）、声音设置都落定后，走与用户点开关同一条 `set_global_enabled` 打开；协议待同意时判定为否，同意后面板再问一次。与启动时恢复配置输入模式是同一思路：后端报事实，前端按用户操作的路径补齐。

WebView 聚焦时 `WH_KEYBOARD_LL` 全局钩子不触发，面板与浮窗都用 `useKeyRelay` 把键盘事件中继到后端 `relay_key_event` 命令，交由引擎统一处理热键 / Toggle 触发 / `pressed_keys` 维护，避免聚焦窗口时热键被吞；同时阻止非编辑区的默认快捷键（F12、Ctrl+Shift+I 等，仅生产构建）。

**AppHandle 不进 packages**：`win-driver` / `win-input` / `win-sysinfo` / `burst-engine` 所有函数均不接受 `AppHandle`。资源目录由 `commands/driver.rs` 从 `app.path().resource_dir()` 取得后传入，Tauri 状态管理留在 commands 层。

## Crate 依赖图

```
packages/crypto      packages/migrate
         ↑                ↑   ↑
         └────────────────┘   │
    packages/qzh-format       │
         ↑                    │
    packages/qzh-profile ─────┘
         ↑
    apps/main/src-tauri

packages/win-input  ← packages/burst-engine ← apps/main/src-tauri
packages/win-driver / win-sysinfo / resource-integrity ← apps/main/src-tauri

apps/site（独立 pnpm 项目，消费私有制品 @xwink/ui；引用 apps/main 的 theme.ts 与 markdown-parse.ts）
```

## 输入模式与注入通道

`win_input::dispatch(KeyId, is_up)` 是统一入口，按 `(mode, KeyId)` 分发到对应后端。四档通道由用户在设置中选择：

| 模式                           | 实现                                                    | 说明                                                                                                                                                                                                                                         |
| ------------------------------ | ------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| SendInput（通用模式，默认）    | `win-input/src/lib.rs`                                  | 键盘 `SendInput INPUT_KEYBOARD` + `KEYEVENTF_SCANCODE`；鼠标 `INPUT_MOUSE` + `MOUSEEVENTF_*`（X1/X2 用 `MOUSEEVENTF_XDOWN/UP` + `mouseData=XBUTTON1/2`）。`dwExtraInfo = SIM_MARKER` 标记自身注入事件防循环                                  |
| Interception（游戏模式，主推） | `win-input/src/interception.rs`                         | 键盘 + 鼠标设备各扫描一次，鼠标状态位映射 `INTERCEPTION_MOUSE_BUTTON_4/5_DOWN/UP`（X1/X2）。`interception_send` 返回写入 stroke 数，鼠标 / 滚轮失败回退 SendInput。需要安装驱动并重启，以管理员运行                                          |
| DDSimple（DD驱动，备用）       | `win-input/src/ddsimple.rs` + `dd_common.rs`（dd63330） | 键盘 `DD_key`，鼠标走 `MOUSE_INPUT_DATA.ButtonFlags`，原生支持 X1/X2 侧键；滚轮用 DD SDK 上滚 / 下滚编码。无需重启但需管理员。`ExtraInformation` 被驱动写死为 0，`SIM_MARKER` 无法幸存，自注入回灌改由 `PENDING_INJECTIONS` 时间窗口队列过滤 |
| HidStick（外设模式）           | `win-input/src/hidstick.rs`（`serialport` crate）       | 挨个 USB 串口握手（`ID` 回 `ID xwink-hand …` 才算），跳过 CH34x / CP210x / FTDI / PL2303 等转串口芯片（常把 DTR 接到开发板复位脚，握手一拉就复位）、乐鑫的口先试；不认 VID/PID，PID 是固件自定的。一行一条指令（`KD`/`KU` 带 HID usage、`MD`/`MU`、`MW`）、逐条等 `OK`，协议以 xwink-body 仓 `firmware.md`「HID 棒固件」为准。即插即用、无需管理员。外设驱动敲出的是真硬件事件，自注入同 DD 靠 `PENDING_INJECTIONS` 过滤；不在 HID 键盘页的键（媒体键）逐条回退 SendInput |

X1/X2 在 DD 模式 / 鼠标设备缺失时按 once 旗标 warn 一次后自动回退 SendInput。`dd63330.dll` 纳入打包资源、运行时完整性校验（`packages/resource-integrity`）与发版前 `pnpm check:resources`。

**DD-HID 已永久移除**（驱动不稳定会导致蓝屏）：注入后端、`InputMode::DdHid`、安装链路与 `ddhid.63340.dll` 全部删除，`InputMode::from_str("dd_hid")` 返回 `None`，存量用户的 `input_mode` 配置在 `collect_configured_input_mode` 自动回落 SendInput。仅保留卸载与残留清理：`win_driver::dd_hid` 的检测与 `uninstall`、`uninstall_dd_hid_driver` 命令、`repair_dd_hid_residue`、诊断报告导出，以及随包分发的 `ddhid-driver/`（`ddc.exe -u` 要用）。

**外设驱动连接**：后端只报事实——插拔监视线程（`bootstrap/input.rs` + `hidstick::StickTracker`，后台每 1.5 s 枚举一次，只对新出现的口握一次手）、外设模式下的链路断开（心跳或下发失败，`win_input::set_on_hidstick_offline`）与手动连上（`set_input_mode` 成功）汇到同一个连接态，翻转时发 `hid-stick-connected` / `hid-stick-disconnected`。自动握手只碰乐鑫 VID 的口（`pick_auto_candidates`）、不在启动主线程上做：用户没点外设模式时不能拖慢启动，也不能去拉别家设备的 DTR（Arduino Uno 一类会被复位）；手动选外设模式与诊断才按 `pick_candidates` 握全部非转串口芯片的口。外设模式正用着的口不再握手；链路断了而口还在时监视线程忘掉它、下一轮重握。断开的顺序固定为「win-input 回退 SendInput 并另起线程释放旧口 → 报断开」：口不释放，插回来常拿到同一个口名却打不开；监视线程先看到口消失时也经 `hidstick_link_down` 走同一条路径。刷新状态、提示、问要不要切过去都是面板的事：不在外设模式时收到「连上」弹确认（启动时本就插着的那一下可能早于面板开始监听，面板启动落定后自查一次），同意后走 `selectInputMode`。外设驱动拒收某个按下（六键已满等）时这一下改走 SendInput，它的松开也必须走 SendInput（`HIDSTICK_SENDINPUT_DOWN`），否则系统收不到抬起、键卡住。卸载 / 修复驱动时 `leave_driver_backend` 不动外设模式。防卡键两端都做：固件在 DTR 掉或按着键 1 s 无指令时松开全部，这边释放后端时发 `RESET`；心跳只在没按着键时发，否则会替卡死的引擎续命、废掉固件那道保险。

**外设驱动与 Interception 设备槽**：Interception 的 `keyboard.sys` / `mouse.sys` 是键盘类 / 鼠标类的上层过滤驱动，每类 10 个设备槽，按设备出现先后往后分配，拔掉不还、重启才清零（实测鼠标槽 12、14–17 空着，新插的仍分到 18）。某类槽满后再插进来的设备分不到槽，输入被过滤驱动吞掉，设备管理器却显示正常——外设驱动对每条指令照回 `OK`，低级钩子一个键也收不到。装卸 DD-HID 每次都会留下一个虚拟键盘占槽。诊断修复的「外设驱动」两项（`repair.rs` 的 `diagnose_hidstick`）：串口扫描对每个非转串口芯片的 USB 串口握一次手（外设模式正用着的口不碰），设备槽检查用 `interception::device_hardware_ids`（只读、不要管理员）按 VID/PID 找外设驱动的键盘与鼠标各占几号槽，缺哪个报错并建议重启。**别用「移除全部键盘再重扫」去清槽**：实测 `keyboard.sys` 不会因此卸载，重新加回的键盘（包括用户的实体键盘）全都分不到槽而失灵，只能重启。

**输入模式与布局互斥**：DDSimple 与 HidStick 的单键规则约束无法用横版键鼠图表达（横版每个键位都是 `trigger == target` 的重合态，DD 下重合态切换连发是空集，见 [key-policy](key-policy.md)），故二者互斥——`selectInputMode`（横版下禁选这两档）与 `switchLayout`（这两档下禁切横版）两个 choke point 拦截并提示。后者按后端下发的 `coincident_toggle` 能力位判断而非模式名，且用 `aria-disabled` 而非 `disabled`：禁用的按钮不触发 `onClick`，原因提示就永远跑不到。

## 连发引擎（`packages/burst-engine`）

`windows_sys` `WH_KEYBOARD_LL` + `WH_MOUSE_LL` 双低级钩子共用同一消息循环线程，监听键盘与鼠标 5 键（左 / 右 / 中 / X1 / X2，含 `WM_XBUTTONDOWN/UP` 高 16 位识别 X1/X2）及滚轮（`WM_MOUSEWHEEL`，每格瞬发 press+release）。

- 注入事件先在 hook 层过滤：SendInput / Interception 用 `SIM_MARKER`，DDSimple 用 `PENDING_INJECTIONS`（以 `(KeyId, is_up)` 为键）。
- 引擎用 `pressed_keys: HashSet<KeyId>` 记录已按下的物理键，只让首次 down 进入 `on_key_press`，up 时移除；不依赖 `KBDLLHOOKSTRUCT.flags` 的保留位判断 key-repeat。
- 线程编排：`catch_unwind` 包裹引擎线程，panic 后记录日志并补发释放事件；并发连发用 `AtomicBool cancel + thread::park_timeout`，`Drop` 时先 signal 再 join 确保按键不卡住。规则热更新前停止连发线程并清空 toggle 状态。
- 互斥组（`BurstRule.group`）：同组同一时间只有一条规则在跑。切换规则开启时停止同组其他切换规则；同组切换时只播报「新规则开始」。
- 组内插队栈：组内长按规则按下时插队——同组正在跑的规则标 `paused`（仍留在活跃集合）并让调度器停下，长按入该组的 `hold_stacks` 栈开跑；松开时先停自己，再把组还给栈里仍按着的上一条长按，否则还给被暂停的切换规则。栈中间先松的只出栈，不影响栈顶。插队期间开启的同组切换规则以 `paused` 登记，只成为松手后的恢复目标——正在按住的长按不能被挤掉，否则手指按着却不连发。滚轮触发的长按每格只是一次点按、没有「按住」，不参与插队。全局暂停、换规则、切后端、退出时清空栈与暂停状态。启停命令在 runtime 锁内发给调度器：面板聚焦时键盘走 relay 命令线程、鼠标走钩子线程，锁外发送会让命令到达顺序与状态变更顺序相反，留下一条引擎以为已停的连发；发送只是无界通道入队，不阻塞钩子线程。
- `get_active_ids` 返回运行 ∪ 暂停：前端靠它差分播报，暂停 / 恢复不改变集合，插队不会多响；`get_rule_states` 返回 `RuleStates { running, paused }` 供界面区分两种状态。
- 多规则隔离：模拟目标键不会触发其他规则的启动 / 停止逻辑。
- 非 Windows 平台提供空实现（`cfg(windows)` 隔离）。

**总并发限速**：连发间隔结构下限 1ms（旧配置兼容），有效下限 `MIN_EFFECTIVE_INTERVAL_MS = 10ms`——全局 LL 钩子链 + RIT 对每个注入事件有固定「过路税」，管线可持续的总注入速率有上限。加载时 `Profile::clamp_intervals()` 把 <10ms 静默钳到 10ms（所有读取入口经 `load_from_path` / `read_profile_from_file` 这一处）。运行时 `process_due` 按「基础下限 × 当前活跃规则数」放大每条规则的有效下限，使总 tap 速率 ≈ 1000/基础下限、与规则数无关，避免多规则叠加超发导致停止后「收不住」。只拉长拍间间隔、不改 `hold` 点按时长。

## 全局热键

`global_toggle` / `global_stop` / `panel_toggle` 不走 `tauri-plugin-global-shortcut`，与连发规则共用 `burst-engine` 低级 hook：热键检测优先于规则处理，且不受 `global_enabled` 当前状态限制，启动期立即生效。只允许绑定键盘实体键、三者互不重复——在绑定 UI 拦截（`KeyCapture` 按槽位允许集收键 + `SettingsDialog` 去重校验），避免同一键被某个热键抢先处理导致其余功能失效。热键可绑修饰键（左右 Shift / Ctrl / Alt / Win），连发规则不可。

AltGr 布局（德语 / 法语 / 波兰语等）把右 Alt 当 AltGr，键盘驱动会在它前面补发一个左 Ctrl，低级钩子看到两个独立事件，因此右 Alt 与左 Ctrl 同时绑热键会互相牵连——不做代码过滤（合成 Ctrl 与物理 Ctrl 在钩子层不可靠区分），改由 `SettingsDialog` 在绑定右 Alt 后按是否也绑了左 Ctrl 分两级提示。

## 前端结构要点

- 面板根组件 `PanelApp.tsx`：竖版规则列表 + 横版键鼠图（`HorizontalLayout.tsx`，键位表在 `keyboardLayout.ts`），布局各自定设计尺寸（竖版 405×720 即 9:16、横版 1072×603 即 16:9，`windowScale.ts` 的 `LAYOUT_BASE`），设计尺寸即最小尺寸。
- 面板窗口等比缩放（前端 `useWindowScale.ts`，纯计算在 `windowScale.ts`，断言在 `scripts/window-scale.test.ts`；锁比例在 `commands/window.rs`）：窗口可拖边，设计尺寸即最小尺寸。宽高比在后端锁：`set_panel_aspect` 给面板窗口挂 subclass 处理 `WM_SIZING`，拖边的每一帧按设计比例改写系统拟定的窗口矩形。以 `WM_ENTERSIZEMOVE` 时的窗口中心为准向四周缩放：被拖的边到中心的距离是半个窗口，拖左右边以宽为准、拖上下边以高为准、拖角取较大者；摆好后越出拖动开始时那块屏幕的工作区就整体推回，中心让位于不出屏，否则放在角落的窗口几乎放不大。上限是那块屏幕工作区装得下的最大等比尺寸，最小尺寸每帧按窗口 DPI 现算。拖标题栏移动不受限。Tauri 没有封装宽高比，事后吸附又只能靠「静止若干毫秒」猜松手，手感别扭，所以放在 `WM_SIZING` 里；矩形换算是平台无关的纯函数 `resize_centered`，单测在 `window_tests.rs`。贴靠分屏、Win+←/→ 这类系统直接摆的尺寸不经 `WM_SIZING`，在 `WM_WINDOWPOSCHANGING` 里按比例收进系统给的矩形并居中（`fit_within`）；比例在舍入误差内（面板自己 `setSize` 的尺寸）或比设计尺寸还小（最小化）时原样放行。macOS 用系统自带的内容比例约束，不做中心缩放。最大化由前端模拟（`maximizable: false`）：比例锁着，系统最大化铺满工作区会变形，所以取工作区装得下的最大等比尺寸并居中（`maximizedRect`），期间 `setResizable(false)` 禁止拖边、resize 不写 `panelScale` 也不广播给浮窗。拖边拉到最大也算最大化：后端在拖过边的 `WM_EXITSIZEMOVE` 时发 `panel-resize-ended`，前端比对窗口宽度与最大化矩形，到了就转成最大化（`reachedMaximum`；工作区装不下比设计尺寸更大的窗口时最大化矩形就是最小尺寸，不算）——只在松手时判断，拖动中途禁止拖边会把正在进行的拖动掐断。不管怎么进的最大化，还原都回到 100% 并居中，不记最大化前的位置。最大化状态存 settings 的 `panelMaximized`，启动和切换布局时都保持。标题栏不用 `data-tauri-drag-region`，因为它的双击会走系统最大化，拖动也不会先还原。改由 `handleHeaderMouseDown` 处理：双击切换；最大化时拖出几像素才按鼠标在窗口里的相对位置缩回 100%（`restoreUnderCursor`），再 `startDragging`。前端在 resize 时按宽高比例里较小的值设 webview 缩放（`setZoom`），停下 400 ms 后把比例按布局存进 settings 的 `panelScale`；启动与切换布局时按该比例定尺寸并居中。用 webview 缩放而不用 CSS `zoom`，是因为前者等同浏览器缩放，`getBoundingClientRect` 与鼠标坐标都在同一套 CSS 像素里，走线与引导气泡的定位不用改。最小化时 Windows 报 0×0，不触发尺寸校正；系统文字大小热切换若改变无边框客户区，前端同时监听视口 resize，按设计比例与最小尺寸校正非零异常尺寸。比例上限按当前布局各自的设计尺寸算，竖版多半被高度顶住、横版多半被宽度顶住，两个比例分开存，所以不取两者的较小值。浮窗不能拖边（宽度随激活规则数自动变化，整块又是拖动区），跟随面板当前布局的比例：面板在比例停稳后广播 `panel-scale-changed`，浮窗启动时读 settings 的 `layout` 与 `panelScale` 取初值；浮窗设同样的 webview 缩放，窗口尺寸按内容 CSS 尺寸乘以比例；浮窗同时监听内容和视口变化，比对实际原生客户区后才调整窗口，避免文字大小热切换后残留底部裁切及重复 resize。
- Windows 文字缩放隔离：`bootstrap/webview_scale.rs` 在面板和浮窗的 WebView2 控制器上关闭自动 RasterizationScale 检测，设置 RAW_PIXELS 边界及实际窗口 DPI 倍率，避免 Windows 辅助功能的文字大小被额外乘入固定画布。原生 ScaleFactorChanged 事件按所在屏幕更新倍率，因此跨屏 DPI 仍生效；前端的应用 ZoomFactor 和窗口尺寸计算保持独立。设置失败写日志，不改变用户系统设置。
- 竖版是一个统一列表：按规则数组顺序渲染，分组在其第一个成员的位置整体渲染为 `RuleGroup`（组头语义句 + 运行 / 暂停状态区），单卡是 `RuleCard`（纯展示 + 回调，不碰 `invoke`）。筛选只影响显示、不持久化；会改变分组成员的写入都经 `pushGroupMove`，某组第一次同时含长按与切换时提示一次插队语义。
- 横版悬停走线：`hkbWires.ts` 是纯函数（边集合、曼哈顿 + 45° 切角路由、出线侧选择），`WireOverlay.tsx` 只量键帽位置画 SVG；几何断言在 `scripts/hkb-wires.test.ts`（`pnpm test:ui`）。路由以悬停键为中心：跨行走朝向对方那一侧，同行在上 / 下里穷举取交叉最少的（同样少时出线在上、入线在下）；同侧同向按跨度嵌套——同行的远者在外层，跨行的远者在内层，因为跨行末段要穿过所有通道。不从左右侧边直连相邻键：键距仅 5px 看不清，形态也不统一。同侧出线点排不下时压缩间距，平行段贴近到 3.5px 内也计作冲突，促使穷举把多出的线分到另一侧。横版一屏展示完整、容器 `overflow: hidden` 不滚动，走线坐标直接相对容器量取，悬停只在窗口缩放时收起。
- 设计 Token 在 `theme.css`，主题预设色板与亮 / 暗 / 跟随系统在 `theme.ts`（写 `data-theme` 到根元素）。所有组件通过变量取色取尺寸。
- 对比度按 @xwink/ui 规范：文字在所有会落到的底色上过 WCAG AA 4.5:1。底色包括三种 surface 和叠满主色淡染的面板底（暗色再加卡片半透明层），淡染浓度统一走 `--fb-app-tint-*`，上限是 `theme.ts` 的 `APP_TINT_MAX`；超过这个值，最弱的 faint 档灰字就会掉线。`theme.ts` 的 `deriveThemeVariables` 按主色与生效明暗推导以下几项，再内联注入：实底前景 `--fb-on-primary`（在白与近黑里按对比度取高的，绯粉、流金等亮色用深字）、hover / active（沿 OKLCH 明度往离开前景的方向走）、主色当文字用的 `--fb-color-primary-text`（亮色档加深、暗色档提亮到过线）。所以主色当文字一律用 `--fb-color-primary-text`，语义色当文字用 `--fb-color-{success,danger,warning}-text`，不直接拿基色。颜色算法在 `contrast.ts`，与 `@xwink/ui/brand` 同源；没有直接依赖那个包，是因为发版工作流不带私有源凭据，且 Node 测试不剥离 node_modules 里的 TS。产品页 `apps/site` 直接用包里的 `brandVariables`，与应用的前景规则一致。
- `components/Overlay.tsx` 提供锚定定位（12 方位 + 视口钳位）与遮罩；Toast / ConfirmDialog / ContextMenu 都建在它上面。层级 Token：`--fb-z-overlay-*` 供弹窗，`--fb-z-tour-*` 供新手引导（要叠在设置弹窗上）。
- 新手引导 `tour/`：`TourRunner.tsx` 是唯一有 DOM 逻辑的文件（四块遮罩镂空、锚定气泡、实操判定），教程内容在 `tour/tours/<id>.ts`，只经 `TourHost` 接口操作宿主（含互斥组教程用的 `createSampleGroup` / `deleteSampleGroup`，快照带引擎的 `runningRuleIds` / `pausedRuleIds` 供实操判定）；进度存 `settings.json` 的 `tours` 键：`completed` 是学过的教程，`introVersion` 是已自动展示过的首启教程版本——与用户协议同一套语义，`TOUR_INTRO_VERSION`（`tour/types.ts`）一 bump，老用户下次启动会再自动看一遍。判定刻意不看「有没有规则」：新装配置自带两条未启用的出厂规则，那个口径对新用户永远为假。说明书大纲在本 skill 的 `references/manual/<id>.md`，`pnpm skills:check` 钉住一一对应。
- 剑网三专栏（☰ 菜单 → 剑网三专栏，`dialogs/Jx3ColumnDialog.tsx`）：数据格式与校验在 `jx3/column.ts`，界面用的纯计算在 `jx3/view.ts`，断言在 `scripts/jx3-column.test.ts`。数据只有一份 `apps/site/public/jx3/column.json`，产品站发布后的地址是 `https://app.xwink.fun/flair-bloom/jx3/column.json`；后端 `commands/jx3.rs` 启动时后台预取（`prefetch`），按「线上 → `{app_data_dir}/jx3-column.json` 缓存 → 随包快照（`include_str!` 同一个文件）」回落，线上 `schema` 比应用新时也回落并提示更新。取数排队（`Jx3State.loading`）：预取、打开、刷新同时到时依次取，不刷新的请求复用前一个的结果，状态里留下的总是最后返回给界面的那份，外链放行与界面一致。后端只认格式版本与骨架，前端拿到后再跑 `validateColumn`，不合格就提示而不渲染；产品站发布流程先跑 `scripts/jx3-column.test.ts`，坏数据发不出去。应用不直接调 JX3Box：它的接口没有文档，一改格式所有已装客户端都会坏；经产品站中转，坏了改数据重发站点即可。外链只走 `open_jx3_link`：协议是 https 且地址在后端当前持有的数据里，前端被注入脚本也只能打开列表里的链接。条目带授权记录（`authorized_at` / `authorized_note`）的常驻展示，不带的公开帖折叠并标来源；宏正文只允许出现在授权条目里，`validateColumn` 拦下。门派 id 与主题预设 id 一致，打开时按主题色默认选中。
- 冲突检测 `conflicts.ts` 管跨绑定的软性提醒（面板键遮蔽全局键、热键盖住规则触发键、A 的连发键是 B 的触发键），与后端的硬性录入策略互不替代。

## 数据存储路径

| 数据               | 路径                                                                   | 说明                                                       |
| ------------------ | ---------------------------------------------------------------------- | ---------------------------------------------------------- |
| 配置文件（`.qzh`） | `{app_data_dir}/profiles/`                                             | 导入 / 导出经文件对话框，不暴露内部路径                    |
| 自选提示音         | `{app_data_dir}/sounds/`                                               | 导入时复制进来，settings.json 只存文件名                   |
| 应用设置           | `{app_data_dir}/settings.json`                                         | `tauri-plugin-store`；复用 `packages/migrate` 迁移基础设施 |
| 兑换码许可证       | `{app_data_dir}/licenses/<码哈希>.jws`                                 | 一张码一份 billing 签发的原文，可叠加；每次查询重新验签    |
| 剑网三专栏缓存     | `{app_data_dir}/jx3-column.json`                                       | 上次从产品站取到的列表与取到的时间，取不到线上时用         |
| 待安装更新包       | `{app_local_data_dir}/pending_update/`                                 | 静默下载完成后暂存                                         |
| 日志               | `{app_log_dir}/`（Windows `%LOCALAPPDATA%\fun.xwink.flairbloom\logs`） | 按天滚动，保留 7 天，`cleanup_old_logs` 清理               |
| 崩溃日志           | `{app_log_dir}/crash-{unix_ts}.log`                                    | panic hook 写入                                            |

路径由 Tauri `PathResolver` 跨平台解析；日志目录由 `bootstrap/logging.rs` 显式定义。卸载时用户配置、设置与日志保留（Windows 惯例），注册表自启动项与「以管理员模式启动」的兼容性标志（`HKCU\...\AppCompatFlags\Layers`，值名是 exe 路径）由安装器清除。

## 日志与崩溃

- Rust 端 `tracing` + `tracing-subscriber` + `tracing-appender`，结构化格式（时间戳、级别、模块、线程 ID），默认 INFO。分级：ERROR 功能不可用（引擎崩溃、文件损坏）；WARN 降级运行（重试成功、配置回退）；INFO 关键状态变更（连发启停、配置切换、教程开始 / 完成）；DEBUG 每次按键事件、IPC 消息，默认关闭。
- 前端 JS 错误经 `log_from_frontend` 命令转发到同一日志文件。
- `std::panic::set_hook` 写独立崩溃日志并弹 rfd 原生对话框（独立线程展示、进程级去重）：panic 可能发生在 LL hook 等敏感线程，原地阻塞会拖垮系统输入；正文直接展示崩溃日志完整路径，不单设「复制路径」按钮。
- 关于 / 诊断入口可打开日志、数据、安装、驱动目录（`open_app_dir` 白名单）。日志不含硬件 ID、用户名。

## 容错策略

| 场景                         | 策略                                          |
| ---------------------------- | --------------------------------------------- |
| 连发循环线程 panic           | `catch_unwind` 捕获，记录日志，补发 `key_up`  |
| 配置文件损坏 / 篡改          | 提示用户，提供恢复默认配置，不阻塞启动        |
| 自动更新下载失败             | 前端提示，不影响使用                          |
| 全局监听或输入后端初始化失败 | 提示切换模式 / 以管理员运行，其余功能保持可用 |
| 规则热键冲突                 | 界面标注并提示                                |
| Tauri Command 异常           | 所有 Command 返回 `Result`，前端统一 toast    |

## 技术选型

| 用途                               | 库 / 工具                                                                       |
| ---------------------------------- | ------------------------------------------------------------------------------- |
| 全局键鼠监听                       | `windows_sys` `WH_KEYBOARD_LL` / `WH_MOUSE_LL`                                  |
| 按键 / 鼠标模拟                    | `windows_sys` `SendInput`；Interception；DD SDK（dd63330）                      |
| 自动更新                           | `tauri-plugin-updater`                                                          |
| 配置加密 / 密钥派生 / 许可证       | `aes-gcm`、`hkdf` + `sha2`、`ed25519-dalek`、`base32`                           |
| 应用状态持久化 / 开机自启 / 单实例 | `tauri-plugin-store`、`tauri-plugin-autostart`、`tauri-plugin-single-instance`  |
| 前端                               | React + TypeScript + 原生 CSS，Vite 多入口（`panel.html` / `panel-float.html`） |
| Monorepo                           | Cargo workspace + pnpm workspaces                                               |

## 已确认的风险与对策

| 风险                                        | 对策                                                   |
| ------------------------------------------- | ------------------------------------------------------ |
| 低级 hook 与模拟输入自循环                  | `SIM_MARKER` + `PENDING_INJECTIONS` 过滤               |
| 反作弊拦截模拟输入                          | 不做技术规避，EULA 与文档明确说明                      |
| AES 密钥被逆向提取                          | 接受「防普通用户」定位，不对抗专业逆向                 |
| 系统时间回拨绕过许可证                      | payload 含 `issue_time` 做下界校验                     |
| 更新包被中间人替换                          | Tauri updater 强制 `.sig` 签名验证 + HTTPS             |
| 驱动残留                                    | 诊断修复提供安装前置检查、残留识别、深度清理与重启提示 |
| WebView 聚焦吞掉热键                        | 前端中继键盘事件到后端                                 |
| 坏版本经静默更新即时铺开且 updater 不可降级 | 见 [release](release.md) 的护栏与应急回退              |
