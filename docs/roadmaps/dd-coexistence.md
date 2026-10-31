# DD 与其他按键助手共存排查

状态：2026-10-31 完成源码、随包 DLL、三个对照样本的静态检查及回灌算法的条件性实验；尚未复现原反馈。产品代码未修改，不能据此将责任归给某一方。

## 反馈边界

用户报告使用 FlairBloom 后其他连发助手无法使用，并指出 DD 旧版本失效是已知问题。无法确认用户使用的 FlairBloom 版本、输入模式、是否同时运行，以及是否执行过驱动卸载／深度清理。

用户另提供本机极简、千机，以及最新版 TheCelestialDiviner 作为对照样本；它们未被确认为原反馈涉及的软件。

## 已确认事实

1. 当前 DD 模式加载随包 `dd63330.dll`，调用 `DD_btn(0)` 初始化；正常使用该模式不经过 DD-HID 63340 的安装／卸载入口。DLL 不只是用户态键鼠函数：静态可见创建、启动 `dd63330` 服务及打开驱动设备的路径。
2. 随包 DLL SHA-256 为 `72FBA9C7721AC394A0803D0C97835B0DA21C9D6DDA8076FC0584A566CD7B2A9D`。在默认 ImageBase `0x180000000` 下，`DD_btn(0)` 进入 `0x180003430`；初始化路径在 `0x180003981` 调用 `CreateFileW`，文件名为 `\\.\dd63330`，访问权限 `0xc0000000`，`dwShareMode=0`，`OPEN_EXISTING=3`。这表示 DLL 请求不共享设备访问；另一助手若使用同一设备，需要优先验证占用冲突。尚未进行两个真实客户端的运行对照，不能宣称已复现。
3. `packages/win-input/src/lib.rs::init_backend` 只有显式进入 `SendInput` 分支才 `take()` 释放 DD 后端。DD → Interception、DD → HidStick 均保留原 DD 后端；目标后端初始化失败而把 `CURRENT_MODE` 改为 SendInput 也不等同于执行释放分支。因此界面不在 DD 模式时，进程仍可能持有 DD DLL／其内部资源。暂停全局连发同样不会卸载后端。
4. `DdFfi::drop` 调用 `FreeLibrary`。这确认 Rust 的释放入口存在，但仅凭这一调用不能保证第三方 DLL 已完整清理内核服务；需要运行证据。彻底结束进程与隐藏主面板／收进浮窗不同。
5. 当前键盘、鼠标低级钩子均将事件传给 `CallNextHookEx`，自注入过滤也继续传递事件；未发现通过低级钩子普遍吞掉其他助手按键的路径。
6. DD-HID 卸载与深度清理针对系统级 `ddhid63340` 服务、设备与 OEM INF，不识别“由哪个助手安装”。如果其他助手依赖同一驱动包，执行这些功能确实可能影响它；这条路径需用户显式调用，不是普通启动时自动清理。

## 旧版本问题的证据与限制

- [DD 作者仓库 README](https://github.com/ddxoft/master)：免费版加载时需要联网认证。因此初始化失败除版本外，还可能涉及认证或网络，不能把失败统一判为“被另一助手破坏”。
- [上游 issue #72](https://github.com/ddxoft/master/issues/72)：2023-09-07 已有“旧版本 DD 初始化失败，DD_btn(0) 返回 -7”的报告。这是独立故障记录，未提供所有旧版失效的版本边界，也未证明本次用户使用该版本。
- [Microsoft CreateFileW 文档](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)：共享模式为 0 的语义是不允许后续冲突访问。应用到具体 DD 驱动的共存表现仍需实际验证。

## 后续判别

| 对照现象 | 优先检查；不是单凭现象即可定因 |
| --- | --- |
| 彻底退出 FlairBloom 后，重开对方助手恢复 | 运行期设备占用、模式切换后的 DD 资源保留 |
| 重启且不启动 FlairBloom，对方仍失败 | 对方 DD 版本／认证、系统兼容、杀软、此前执行过的系统驱动清理 |
| 对方更新后恢复 | 支持版本／兼容差异，但仍需同机启动顺序对照，不能排除占用变化 |
| 执行过 FlairBloom 的 DD-HID 卸载／清理后才失败 | 共享驱动包是否被卸载，不能只查旧版 DLL |
| 安装游戏模式驱动并重启后，对方 DD 初始化成功但无输出 | Interception 键盘过滤链与 DD 兼容；退出本应用不会卸载系统过滤驱动 |

本机只做静态检查，没有安装／卸载／启动内核驱动或修改用户设置。本地反汇编脚本与输出在 Git 忽略目录 `.playwright-mcp/dd-analysis/`。优先工程修正方向是离开 DD 模式时统一释放旧后端，并补充资源持有／释放及初始化错误日志；未实现、未宣称已解决该反馈。

## 对照样本与版本对应

| 样本 | 已核实 | 限制 |
| --- | --- | --- |
| `D:\Green\极简按键v1.1.81\极简按键v1.1.81.exe` | 文件版本 1.1.8.1，.NET 压缩／保护；随附 `DD.dll` 产品版本 **DD54900**，设备名 `\\.\dd54900` | 主程序只有受保护加载器可见，未确认停止实现；设备编号不能直接排序为驱动新旧 |
| `D:\Green\千机\千机魔盒.exe` | 文件版本 2.2.1.94；PE 含 `.themida`、`.vm_sec` 保护段 | 未确认其实际 DD 版本、输入源区分与停止实现 |
| `TheCelestialDiviner-1.9.1-win64.zip` | .NET 程序含 `EchoGuard`、输入钩子与调度器；随包 Interception，不含 DD DLL，DD 通过外部获取 | 未运行程序，不能证明本机实际选择了 DD 模式 |

极简作者的 [使用说明](https://weibo.com/ttarticle/p/show?id=2309404272652092059257) 明确描述“单键触发”再次按同一键停止；本地更新日志也有暂停识别异常修复。支持该交互是事实，不能据此证明没有停止失败。

TheCelestialDiviner 源码取自 [用户提供的 Gitee 仓库](https://gitee.com/sofqi/The-Celestial-Diviner/)，固定提交 `c2faa67e90b1229f687a4722281607eb38914b82`（项目版本 1.9.1）。压缩包程序内嵌 CHANGELOG 与该提交的文件 SHA-256 完全一致：`576F5414EBA593CDC7C2E06CB2E809AE03C026AFD1325BECBB691CF54C5CE291`。这是版本对应证据，不等同于完整二进制可复现构建证明。以下链接使用作者的 GitHub 镜像同一提交。

## TheCelestialDiviner 如何处理同键启停

1. **预登记回灌名额**：`DdDriverService.cs:940` 在调用 DD 前按 VK 和方向登记记录，失败后撤销，避免回灌先于函数返回造成漏记。
2. **FIFO 配平**：键盘仍通过 `WH_KEYBOARD_LL`，`InputHookService.cs:326` 调用 `EchoGuard.TryConsume`。有同键同方向的可接受记录，就过滤本次事件；不是读取到可靠的“来自物理键盘”标记。
3. **自适应窗口及容量限制**：`EchoGuard.cs:44` 每键每方向最多 32 条；`:58` 接受窗口为往返估计的两倍加 40ms，限制 60～600ms。DD 初始估计 170ms、Interception 20ms；消耗记录时更新估计，过旧记录丢弃。170ms 是对方测试环境的观察，不是所有 DD 版本或机器的固定参数。
4. **重复按下与恢复**：`TaskSchedulerService.cs:245` 使用按住表防自动重复，旧记录超过 1500ms 后结合 `GetAsyncKeyState` 决定是否恢复；`:272` 在模式切换时清表。`:317` 全局停止清任务和按住表。
5. **停止执行**：有效的再次按下翻转任务 Active，唤醒等待中的线程；正在进行的一次 Click 完成主键释放后不再继续下一拍。任务间隔等待可以被信号唤醒；不等于可以取消已经阻塞在驱动中的调用。
6. **独立关闭入口与日志**：全局关闭优先于普通任务处理，另记注入耗时、钩子延迟、回灌延迟和丢弃名额。该入口仍位于 EchoGuard 过滤之后；若关闭键也被输出产生名额，并不自动免疫过滤。

参考：[EchoGuard](https://github.com/Dyse-Sofqi/The-Celestial-Diviner/blob/c2faa67e90b1229f687a4722281607eb38914b82/src/TheCelestialDiviner/Services/EchoGuard.cs)、[输入钩子](https://github.com/Dyse-Sofqi/The-Celestial-Diviner/blob/c2faa67e90b1229f687a4722281607eb38914b82/src/TheCelestialDiviner/Services/InputHookService.cs)、[调度器](https://github.com/Dyse-Sofqi/The-Celestial-Diviner/blob/c2faa67e90b1229f687a4722281607eb38914b82/src/TheCelestialDiviner/Services/TaskSchedulerService.cs)。

## 稳定性结论与条件性反例

它选择支持同键 Toggle，并用恢复和诊断降低故障影响。我们当前选择禁止 DD 同键 Toggle，二者是产品容错边界不同，不能解释成“DD 天生绝不可能同键停止”，也不能解释成“竞品已确保可靠区分”。

- 我们的 `SIM_TTL=50ms`。如果本机某条 DD 回灌确实需要 170ms，这条记录已经过期，会把回灌放进物理输入路径；仅把窗口拉长又会扩大吞掉真实按键的机会。需要测生成时间、送达时间、调用耗时与队列深度后判断。
- 对照源码的算法实验 `.playwright-mcp/dd-analysis/echo_model.py`：正常 170ms 回灌能被接受；在“DD 调用报告成功但所有回灌缺失、仍持续 100Hz 登记”的条件下，5 秒内模拟的 20 对物理停止按下／释放全部被新鲜名额消费。停止注入并等记录变旧后，物理输入才通过。这是条件性算法反例，**不是真机故障复现、也不提供发生概率**；它说明队列上限和超时不能保证持续注入时的停止。
- `GetAsyncKeyState` 不是物理来源鉴别器。它读取系统键状态，且低级钩子回调发生在异步键状态更新之前；因此对方源码注释中的“物理键状态”与“可靠区分”不能直接采信。[Microsoft LowLevelKeyboardProc 文档](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc)
- 对方源码头注释声称 FIFO 碰撞守恒足以恢复，而 1.9.1 的 CHANGELOG 明确记录“名额吞真按键、按住表残留、同键停不下来”及修复。应以执行逻辑与实测为准，不能照搬注释里的保证。

## 与“用了我们后其他 DD 助手失效”的关联

作者 [1.9.1 更新日志](https://github.com/Dyse-Sofqi/The-Celestial-Diviner/blob/c2faa67e90b1229f687a4722281607eb38914b82/CHANGELOG.md) 记录了其环境下：装好 Interception 并重启后，DD 初始化成功但注入 F9 的钩子回灌为零；是否创建 Interception 用户态上下文都不改变结果。其产品因此限制两条路线并提示卸载后重启。这是独立项目的同类现场证据，**不应外推为所有系统、所有 DD 版本必然互斥**。

本机只读检查发现键盘类 `UpperFilters={keyboard,kbdclass}`，`keyboard.sys` 与 `mouse.sys` 服务均在运行。这证明本机有 Interception 过滤链，不证明原反馈用户也有，更无法仅凭服务判定由哪个软件安装。我们的游戏模式安装确实调用系统级 `install-interception.exe /install`（`packages/win-driver/src/interception.rs:10`），驱动生命周期超出本应用进程：若反馈用户执行过安装，这条线索比只猜旧 DD 更值得先验证。

后续建议：先补只读兼容诊断与注入／回灌健康检查；让独立停止键避开输出成员；将 DD 同键作为单独试验，不直接取消当前保护。来源识别要用目标 DD 实测确认；Raw Input 能提供设备来源，但不能未经验证就假定 DD 回灌有独立设备可过滤。不要自动卸载用户的 Interception 来“修复”第三方助手。
