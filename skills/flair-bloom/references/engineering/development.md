# 开发环境、目录结构与协作规范

## 常用命令

```sh
pnpm dev                        # 启动 Tauri 开发模式（热重载）
pnpm build                      # 构建生产包（需 FLAIR_LICENSE_PUBKEYS 至少一把公钥；本地不需要兑换时设 FLAIR_LICENSE_ALLOW_NO_KEYS=1）
pnpm lint / pnpm lint:fix       # oxlint 检查 / 自动修复前端代码（范围 apps/main/src）
pnpm format / pnpm format:check # oxfmt 格式化 / 格式检查（CI 用）
pnpm skills:check               # 教程 ↔ 说明书一一对应 + SKILL.md / CLAUDE.md / README.md 相对链接存在
pnpm check:resources            # 发版前打包驱动资源检查

cargo check                     # 检查所有 workspace crate
cargo fmt                       # 格式化所有 Rust 代码
cargo clippy --all-targets --all-features -- -D warnings
cargo test -p <crate>           # 运行指定 crate 的测试，如 -p crypto

pnpm coverage                   # 共享 crate 覆盖率（CI 同源），低于阈值会红灯
pnpm coverage:html              # 浏览器打开 HTML 报告
```

前端类型检查：`cd apps/main && pnpm exec tsc --noEmit`（根目录没有 tsconfig，`scripts/*.ts` 不在类型检查与 lint 范围）。

Windows 构建会把 `interception-sys` 产出的 `interception.dll` 同步到 EXE 同级和 `src-tauri/resources/`，只有内容变化时才复制。重复写入资源会触发 Tauri 文件监听重启；无变化时不写入，也避免覆盖正在使用的 DLL。同步逻辑回归测试：`cargo test -p flair-bloom --test build_resources`。

## 克隆后执行一次

```sh
git config core.hooksPath .githooks
# Claude Code 技能软链（.claude/ 不进版本控制；Windows 需要 MSYS=winsymlinks:nativestrict 才能建真软链）
for n in rust-best-practices tauri-v2 vercel-react-best-practices; do
  MSYS=winsymlinks:nativestrict ln -s ../../.agents/skills/$n .claude/skills/$n
done
```

仓库改过名，两处都吃过绝对路径的亏：`core.hooksPath` 曾指向旧目录导致 pre-commit 一直没跑，`.claude/skills` 软链曾全部失效。都用相对路径。

## 真机验证接法

代理在本机验证界面与交互时，控制的是应用自己的 WebView2（Chromium 内核，讲 CDP），不是浏览器里的页面——纯浏览器打开 `panel.html` 没有 Tauri 的 `invoke`，一加载就崩。操作依据先读 `references/manual/*.md`，写动作一律在新建的测试配置里做（见协作规范）。

1. **起应用**：`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 pnpm dev`，后台跑、stdout 重定向到日志文件。这是 WebView2 官方的附加参数入口，不需要改 Tauri 配置。就绪判定不轮询界面，等日志出现 `FlairBloom started`（`lib.rs` 启动完成的 tracing 行）。**要验需要管理员的路径（游戏模式 / DD 驱动）就在管理员终端里跑这条命令**：应用直接继承管理员权限，`elevated` 为真，不会走「以管理员重启」，调试端口与进程都留在同一权限下。代理会话自身不是管理员时，请用户在管理员终端里起，或整个 Claude Code 从管理员终端启动。
2. **连 CDP**：`http://127.0.0.1:9222/json/list` 返回两个 page，按 url 后缀挑 `/panel.html`（面板）或 `/panel-float.html`（浮窗）。用仓内 `scripts/cdp.mts`（Node 24 内置 `WebSocket`，零依赖）：
   - `node scripts/cdp.mts eval "<js>"`：`Runtime.evaluate`（awaitPromise + returnByValue），可写 `(async()=>{...})()` 做多步，等 React 重渲染用 `await wait(ms)`。
   - `node scripts/cdp.mts key Escape`：`Input.dispatchKeyEvent` 的 keyDown + keyUp，带 `windowsVirtualKeyCode`。必须用它而不是 `window.dispatchEvent(new KeyboardEvent(...))`：合成事件的 target 是 window，capture 与 bubble 监听在 at-target 阶段按注册顺序跑，教程「capture 阶段先行截断」的语义验不出来；真实按键的 target 是聚焦元素。
   - `node scripts/cdp.mts shot out.png`：`Page.captureScreenshot` 落盘后用 Read 看图。
3. **点击**在 `eval` 里 `element.click()` 即可，`HTMLElement.click()` 不受 CSS `pointer-events` 影响，hover 才显形的按钮照样有效。**定位一律用 `[data-tour=...]`**——教程锚点的第二个用途。
4. **收尾**：`Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'flair-bloom.exe' }` 取 PID，`Stop-Process -Id <PID>`，按 PID 不按模式匹配；应用退出后 `pnpm dev` 自行结束，9222 随进程释放。重启只需再跑第 1 步，Rust 已编译好约 10 秒。

5. **伪造后端事件**（验证 `update-ready` 这类靠网络才触发的弹窗）：Vite dev 下在 `eval` 里 `await (await import('/node_modules/.vite/deps/@tauri-apps_api_event.js')).emit('update-ready', payload)`，前端 emit 会回灌到自己的 `listen`；文件名以 `/node_modules/.vite/deps/` 里实际存在的为准，payload 按前端类型的形状给。

坑：

- `.rule-row` 只统计当前筛选下渲染的卡片（被筛掉的组成员折叠成「另有 N 条已隐藏」），数规则数先把筛选切回「全部」。
- **不要让应用在验证中途「以管理员重启」**：relaunch 出来的提权实例不继承远程调试环境变量（CDP 断连），在 `tauri dev` 下还会报 `WebView2 error 0x800700AA` 起不来窗口；普通权限的会话也杀不掉提权进程。需要管理员就按第 1 步从管理员终端起，从一开始就是 elevated。
- **别在 dev 构建上打开「以管理员模式启动」**：该开关把 exe 绝对路径写进 `HKCU\...\AppCompatFlags\Layers`，而 dev 下那个路径是 `target\debug\flair-bloom.exe`，之后 `cargo run` 每次都要求提权、直接失败（`os error 740`）。本机 settings 里 `runAsAdmin` 已为真（用户在正式版里开过）时同样会中招：dev 构建启动时按用户意图自愈，把 debug exe 写进去，第一次能起、下一次就 740，所以验完停进程后顺手删掉。已经中招就删掉该注册表值：`Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers' -Name '<debug exe 绝对路径>'`。

用 MCP 接同一个端口：仓库 `.mcp.json` 里的 `flair-bloom-app` 是 Playwright MCP 以 `--cdp-endpoint http://127.0.0.1:9222` 连应用（新会话首次加载需确认一次）。`browser_snapshot` 看可访问性树、`browser_click` 按元素点、`browser_press_key` 发真实按键、`browser_take_screenshot` 截图，不必知道选择器；应用必须先起、端口先开，重启后重连一次即可。**不要调导航类工具**，那会把面板页面导航走。快照体积大，数规则、读 settings 这类精确取值仍用 `scripts/cdp.mts`。

## 设计准则

- **小白友好，开箱即用**：合理默认值（连发间隔默认 10ms、触发方式默认长按），首启协议同意后进入新手教程，基础视图只显示核心操作、高级选项折叠，报错用自然语言。
- **灵活扩展，充分可配置**：所有行为暴露设置项，`.qzh` 可导入导出，设置面板独立于主配置，功能开关可独立关闭。
- **容错重试，稳健运行**：单个功能出错不影响整体，关键操作失败后自动恢复（策略表见 [architecture](architecture.md)）。
- **日志完善，崩溃可追溯**：用户一键提供有效日志，开发者快速定位。

## Monorepo 目录结构

```
apps/main/src-tauri/src/        # Tauri 后端（Rust）
  lib.rs / main.rs              # 应用入口（薄壳）、窗口创建、事件注册、enter_panel_mode / enter_float_mode
  tray.rs                       # 系统托盘
  bootstrap/                    # 启动期装配（不含 Tauri 命令）
    logging.rs                  # tracing 初始化 + panic hook + 旧日志清理
    agreement.rs                # check_agreement + AGREEMENT_VERSION
    update.rs                   # UpdateLock + 更新检查下载 + 自动更新开关
    profile.rs                  # load_or_init_profile（委托 qzh-profile）
    input.rs                    # init_input_backend + parse_switch_mode_arg
  commands/                     # 前端 invoke 入口（纯 Tauri 桥接）
    app.rs                      # agree_license / check_update / exit_app
    driver.rs                   # 驱动安装卸载 + is_elevated + relaunch_as_admin
    engine.rs                   # 规则 CRUD + 输入模式切换
    import_profile.rs           # 外部配置导入
    log.rs / profile.rs         # 日志 / 配置文件 CRUD
    repair.rs                   # diagnose_environment + repair_* 命令
    resource_integrity.rs       # 打包资源完整性
    ddhid_diagnostic.rs         # 统一驱动与显示环境诊断报告（保留旧 IPC 名）
    display_environment.rs     # 按需采集实时 DPI / 文字大小 / 窗口 / WebView 控制器状态
    sound.rs                    # 自选提示音导入 / 读取 / 删除
    status.rs                   # get_app_status + emit_status_changed
  engine/mod.rs                 # 仅 re-export burst_engine / win_input 公开 API
apps/main/src/windows/panel/    # 面板窗口（React）
  main.tsx                      # 入口，挂载 Provider
  PanelApp.tsx / .css           # 根组件（竖版规则列表 + 横版键鼠图）
  changelog.ts                  # 从随包 CHANGELOG.md 切出指定版本那一节
  conflicts.ts                  # 跨绑定冲突软性提醒
  HorizontalLayout.tsx / .css   # 横版键鼠图布局；keyboardLayout.ts 键位表
  theme.css / theme.ts          # 设计 Token；主题预设色板 + 亮 / 暗 / 跟随系统
  useKeyRelay.ts                # WebView 聚焦时键盘事件中继 Hook（面板与浮窗共用）
  components/                   # UI 基础组件（Overlay、Toast、ConfirmDialog、ContextMenu、KeyCapture、Markdown …）
  dialogs/                      # 弹窗内容组件（设置 / 关于 / 协议 / 更新公告 / 诊断修复 / 导入 / 新手教程目录）
  tour/                         # 新手引导：types.ts、TourRunner.tsx、useTourProgress.ts、tours/<id>.ts
apps/main/src/windows/float/    # 浮窗（panel-float.html）
apps/main/src/assets/           # EULA.md、图标（SVG 源文件 currentColor / 1em）
apps/site/                      # 产品页（独立 pnpm 项目，不在根 workspace，见其 README）
packages/crypto/src/            # aes.rs（AES-256-GCM）、license.rs（billing 许可证 JWS 验签）、kid.rs；build.rs 编入公钥列表
packages/migrate/src/lib.rs     # run_migrations() 泛型迁移运行器
packages/qzh-format/src/        # header.rs（FileHeader）、lib.rs（read_encrypted / write_encrypted）
packages/qzh-profile/src/       # key_id.rs、profile.rs（Profile / BurstRule / OutputStep + validate）、key_policy.rs、storage.rs、schema_migrate.rs、lib.rs
packages/win-sysinfo/src/       # lib.rs（os_version / webview2_version / …）、registry.rs、prereq.rs（HVCI / SAC / 待重启 / Defender 排除）
packages/win-input/src/         # lib.rs（InputMode / init_backend / dispatch / SIM_MARKER / PENDING_INJECTIONS）、ddsimple.rs / dd_common.rs、interception.rs
packages/burst-engine/src/      # BurstEngine + start_listener（LL 钩子 + 消息循环）
packages/win-driver/src/        # elevation.rs、powershell.rs、dd_hid.rs（残留检测与卸载）、interception.rs（install / uninstall）、judge.rs、path_util.rs
packages/resource-integrity/    # 打包资源完整性校验
skills/flair-bloom/             # 本 skill：项目文档单一来源
docs/roadmaps/                  # 变更路线图（active / draft），完成归档到 archive/
scripts/                        # bump-version、extract-changelog、check-skills、check-ddhid-resources、统一环境诊断脚本
```

## 协作规范

- **commit-msg**（Conventional Commits）：`type(scope): description`，type 为 `feat | fix | docs | style | refactor | test | chore | ci | build | perf | revert`，描述中文，不加署名 trailer。
- **pre-commit**（`.githooks/pre-commit`）：暂存 `.rs` → `cargo fmt --check` + `cargo clippy -D warnings`；暂存 `.ts/.tsx` → `oxlint` + `oxfmt --check`；暂存 `.md`、`skills/`、`scripts/check-skills.ts` 或 `tour/` 下文件 → `pnpm skills:check`。
- **Workspace lints**：根 `Cargo.toml` 的 `[workspace.lints.clippy]` 是统一 lint 源（含 `uninlined_format_args = "warn"`）。新增 crate 必须加 `[lints] workspace = true`。
- **覆盖率门槛**：`packages/{qzh-format, qzh-profile, crypto, migrate}` 整体行覆盖 ≥ 85%、函数 ≥ 80%、region ≥ 85%，`.github/workflows/coverage.yml` 在这四个 crate 改动时强制；新增共享 crate 须同步加入 workflow 与 `package.json` 的 `coverage` 脚本。`apps/main/src-tauri` 与 `win-*` / `burst-engine` 因含大量 `#[cfg(windows)]` 代码不在阈值监控范围。`ci.yml` 的 Rust 门禁只在 workspace 成员改动时触发；前端目前无 CI 作业，靠 pre-commit。
- **真机验证用测试配置**：在开发者本机走教程实操、加规则、录键、切驱动模式、清空规则这类有状态的验证，先「新建配置」（名字带 test）并切过去，所有写动作只落在里面；测完切回原配置、删掉测试配置。原因：激活配置是开发者自己在用的，验证顺手改掉的规则与热键不会有人记得恢复。
- 通用协作约定（语言、提交与推送策略、注释风格、危险动作授权）以全局 `~/.claude/CLAUDE.md` 为准。

## 文档约定

- 本 skill 是项目文档单一来源：用户可见行为归 `references/manual/<教程 id>.md`（同时改 `tour/tours/<id>.ts`），架构与工程事实归 `references/engineering/*.md`。`CLAUDE.md` 与 `README.md` 只做索引与首屏，不复制正文。
- 计划：README 末尾「接下来」一节，每条一行，用户视角措辞，不分已定与设想。值得留但未开工的详细设计放 `docs/roadmaps/<name>.md` 标 `Status: draft`。
- 每个改动的规划落 `docs/roadmaps/<change>.md`（Status / 决策 / 问题清单 / 任务卡 / 工期表），完成后打钩、Status 改 completed、移到 `docs/roadmaps/archive/`。
- 只描述现状，不写变更史；关键取舍写一句为什么。

## 其他文档

`docs/` 下只放变更路线图（`roadmaps/`，完成归档 `archive/`）。发版应急回退在 [release](release.md)，测试分层、压力测试与真机清单在 [testing](testing.md)。

- `THIRD_PARTY.md`：第三方组件声明；`apps/main/src/assets/EULA.md`：用户协议。
