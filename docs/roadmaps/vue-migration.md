# 前端迁到 Vue + @xwink/ui — 改造路线图 / Change Roadmap

Status: draft（2026-10-09 立项；2026-10-27 作者定搁置，未开工）
Goal: 面板与浮窗整体换成 Vue + @xwink/ui，视觉语义不变、交互不退化；flair 里通用的组件（新手教程 Tour、提示条 Toast）沉淀进组件库，flair 改用包里的版本。

---

## 1. 背景 / Background

迁移的目的是回灌组件库：flair 作为 @xwink/ui 的一个真实宿主，把组件库没有、而 xwink 各端也要的组件（首先是新手教程）提炼进去。flair 留在 React 上的话，回灌过去的只能是一份移植，flair 自己不用，两份实现会各自漂移。

| 能力                     | 现状                                                                                                                                                         |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 前端规模                 | React 19，`apps/main/src` 86 个文件，约 1.18 万行 TS/TSX + 5000 行 CSS；`panel/PanelApp.tsx` 单文件 3307 行                                                  |
| 前端测试                 | 只有纯函数的 `pnpm test:ui`（`scripts/*.test.ts`：横版走线、Markdown 切分）；没有组件与交互测试                                                              |
| 测试锚点                 | `[data-tour="…"]` 同时是教程锚点与真机测试锚点（`scripts/cdp.mts` 头注释）                                                                                   |
| 与框架无关、可原样保留的 | `conflicts.ts`、`keyboardLayout.ts`、`keyToken.ts`、`hkbWires.ts`、`components/markdown-parse.ts`（产品页也直接引用）、`changelog.ts`、`theme.ts` 的配色计算 |
| @xwink/ui 已有、可直接换 | Button、Tabs、ContextMenu、Dialog、NumberField、Kbd、Progress、CardList、Switch、Slider、Select、Segmented、Tooltip、Popover / DropLayer 定位                |
| @xwink/ui 没有、要回灌的 | 新手教程 Tour（`panel/tour/`，运行器 389 行 + 类型 104 行，宿主快照与宿主接口已隔离成 `TourSnapshot` / `TourHost`）、提示条 Toast（`components/Toast.tsx`）  |
| 业务组件（迁移，不回灌） | 按键录入 KeyCapture（475 行，`window` 捕获阶段 `keydown`）、横版键鼠图与走线、规则卡与分组、9 篇教程内容（`tour/tours/*.ts`）、GroupTimeline                 |
| 组件库宿主形态           | 全部 10 个现有宿主是 Nuxt；@xwink/ui 以 TS 源码发布、依赖里带 `@nuxt/kit`；主题 `useTheme` 支持 `storageKey` / `storageRef`，不必依赖可注册域 Cookie         |

不变量：

- 视觉语义不变：主题色预设（21 个门派配色）、亮 / 暗 / 跟随系统、横竖版布局、状态色（连发中、暂停、停用）含义不变。
- 交互可以优化，不能退化：V0 特征测试里的每条流程迁移后照样通过；键盘可达性、Esc 关闭、焦点归还不能少。
- 更新公告与用户协议的 Markdown 渲染不走 `innerHTML`，链接只显示文本不导航（[updater](../../skills/flair-bloom/references/engineering/updater.md) 的「不引第三方库」两条理由）。
- 前端只通过 Tauri IPC 与后端交互，命令与事件名不因迁移改动。

## 2. 决策 / Decisions

- **D1：一步到位，不留 React / Vue 混合过渡期。** 迁移期间不发版，迁完后整体替换 — 理由：作者定（2026-10-09）。混合挂载要多维护一层状态桥，迁完又要拆掉。
- **D2：视觉语义不变，允许样式小变动；交互可以优化，不能退化。** 退化的判据是 V0 特征测试与真机走查，优化项写进 CHANGELOG — 理由：作者定（2026-10-09）。
- **D3：通用组件按待回灌约定，先在 flair 本地按包的接口写，跑通后整块回灌。** 放在 `apps/main/src/ui-pending/`，回灌发版后改用包里的版本、删掉本地那份；缺口登记在 xwink-console 的 `skills/xwink-engineering/references/todo.md` — 理由：作者定（2026-10-09）。flair 的 9 篇教程是检验接口的真实负载，回灌时只是搬运、不用重新设计。
- **D4：先写特征测试，再动迁移。** V0 的交互测试先在 React 版上跑绿，迁移后必须保持绿 — 理由：现在没有前端测试，没有基线就说不清迁移后「不退化」。
- **D5：交互测试用 Playwright 的 `msedge` 通道，跑 Vite 页面，IPC 用 `@tauri-apps/api/mocks` 的 `mockIPC` 接一个内存假后端** — 理由：WebView2 就是 Edge 内核，同一引擎，免下载浏览器（境外 CDN 走代理很慢）；假后端让测试确定、可在无管理员的环境跑。真后端的链路留给 CDP 真机走查。
- **D6：断言只看结果，不看 DOM 结构。** 选择器只用可访问名、`data-tour` 与少量 `data-testid`；断言看可见文字、焦点、发给假后端的 IPC 调用 — 理由：同一份测试要同时跑 React 版与 Vue 版。
- **D7：视觉对照不做像素门禁。** V0 在关键状态下按亮暗两档截基线图，迁移后生成新旧并排的对照页给作者看 — 理由：D2 允许样式小变动，像素门禁会把每次微调都判成失败。
- **D8：门禁位置。** `pnpm test:ui` 照旧挂在 pre-commit；`pnpm test:e2e` 进 `release.yml`，每张卡验收时本地跑，不进 pre-commit — 理由：交互测试要起 Vite 和浏览器，跑一遍按分钟计，挂在每次提交上太重。

## 3. 问题清单 / Findings

| #   | 问题                                                                                                                                                                                                             | 严重度 | 位置                                                        | → 任务卡 |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ----------------------------------------------------------- | -------- |
| I1  | 没有组件与交互测试，迁移后的行为退化只能靠人工发现                                                                                                                                                               | P1     | `skills/flair-bloom/references/engineering/testing.md`      | V0       |
| I2  | @xwink/ui 的 token 里用了 `color-mix`；flair 的注释说 WebView2 不支持 `color-mix`，所以主题衍生色由 JS 计算后注入。要在目标机器的 WebView2 版本上核实，不成立就删掉这段绕法，成立就在宿主侧给这几个 token 打补丁 | P1     | `panel/theme.ts:1-3`、`@xwink/ui` `styles/tokens.css:42-44` | V1       |
| I3  | 组件库没有纯 Vite 宿主的先例，还连带依赖 `@nuxt/kit`                                                                                                                                                             | P1     | `@xwink/ui` `package.json` 的 dependencies                  | V1       |
| I4  | 状态与视图都塞在 3307 行的单文件里                                                                                                                                                                               | P1     | `panel/PanelApp.tsx`                                        | V3       |
| I5  | Markdown 渲染要保持无 `innerHTML`、链接不导航                                                                                                                                                                    | P1     | `components/Markdown.tsx`、updater 文档                     | V4       |
| I6  | 按键录入在 `window` 捕获阶段监听 `keydown`，可能和组件库对话框的焦点管理、Esc 关闭抢事件                                                                                                                         | P2     | `components/KeyCapture.tsx:446`                             | V3       |
| I7  | 组件库是受限私有源，发版 CI 要能装它                                                                                                                                                                             | P2     | `.github/workflows/release.yml:40`                          | V1       |
| I8  | 教程目录路径被 `scripts/check-skills.ts:7`、`CLAUDE.md` 与 skill 引用                                                                                                                                            | P2     | 同左                                                        | V6       |
| I9  | Tailwind v4 的浏览器基线是 Chrome 111 / Safari 16.4；Mac 版若要支持 macOS 12（Safari 15，Markdown 护栏为它而设），与组件库冲突                                                                                   | P2     | `testing.md`（macOS 12 护栏）                               | 范围外   |

## 4. 任务卡 / Task cards

### V0 — 特征测试基线（React 版） · 进度: 未开始

- **Fixes:** I1
- **Change:** `apps/main/e2e/`：
  - 内存假后端 `fake-backend.ts`：覆盖前端用到的约 45 个 `invoke` 命令与 11 个事件，规则、配置、设置都在内存里。
  - `mockIPC` 注入入口。
  - `@playwright/test` 配置用 `msedge` 通道。
  - 约 20 条流程用例，按说明书 `manual/*.md` 取材：
    - 首启：用户协议，进而自动跑「上手三步」。
    - 9 篇教程各跑完一遍（实操步骤靠假后端驱动；教程本身就串起了大部分功能）。
    - 规则：长按 / 切换增删改；按键录入（键盘与鼠标）；间隔输入边界。
    - 布局：分组与互斥；筛选；横竖版切换，以及 DD 模式拒绝切横版。
    - 配置：新建、重命名、复制、删除、导入预览。
    - 设置：四个页签，热键冲突。
    - 通用交互：右键菜单的键盘导航与 Esc；对话框关闭后焦点归还；提示条；更新公告里的链接不导航；浮窗显示总开关状态。
  - 同一轮在约 12 个关键状态下按亮暗两档截基线图，落到 `e2e/baseline/`。
  - 根 `package.json` 加 `test:e2e`。
- **Acceptance:** React 版全绿；每条用例的选择器只用可访问名、`data-tour`、`data-testid`。
- **Test:** 本身即测试；另把假后端的几条命令和真后端返回的结构对一下（CDP 抓一次真实返回）。
- **Rollback:** 只增测试文件。
- **Depends on:** 无。

### V1 — 地基：Vue + Tailwind v4 + @xwink/ui · 进度: 未开始

- **Fixes:** I2、I3、I7
- **Change:**
  - `apps/main` 加 `vue`、`@vitejs/plugin-vue`、`@tailwindcss/vite`、`@xwink/ui`，两个入口换成 Vue 的 `main.ts`。
  - `.npmrc` 指向制品库，CI 用 Secrets 里的令牌。
  - flair 的主题变量映射到 `--ui-*`，主题色预设经组件库的品牌色入口（`@xwink/ui/brand`）注入；`useTheme` 用 `storageRef` 接 settings.json。
  - 在目标 WebView2 上核实 `color-mix`（I2）。
  - 纯 Vite 宿主遇到的缺口按 D3 记进待回灌清单。
- **Acceptance:** 空壳 Vue 面板在 `pnpm dev` 下能起来，组件库按钮在亮暗与切换主题色时都正确；`pnpm build` 产出两个入口。
- **Test:** `pnpm build`；真机看一眼两档主题。
- **Rollback:** 分支内，不发版。
- **Depends on:** V0。

### V2 — 本地待回灌组件：Tour、Toast · 进度: 未开始

- **Change:**
  - `src/ui-pending/tour/`：通用教程引擎，宿主快照和宿主接口做成泛型参数。
    - 能力：步骤目标按 `[data-tour]` 选择器，没有目标时居中气泡；placement 自动选侧；`prepare` 可返回 `skip`；实操步骤按 `done(now, entered)` 自动前进，带 `actionHint`；按版本号首启自动跑；目录里禁用不可用的教程。
    - 定位用组件库的浮层定位，不搬 flair 的 `Overlay`；聚光遮罩、步骤推进、版本判定拆成纯函数配单测。
  - `src/ui-pending/toast/`。
  - 两者接口按组件库约定写（命名 `XTour` / `XToast`、props 与插槽风格、`gen:ui-meta` 能识别的形态），并把接口草案发给 xwink-console 会话对一遍。
- **Acceptance:** 教程引擎单测覆盖推进、跳过、实操判定、版本判定；组件和 flair 没有任何耦合（不 import `panel/` 下的文件）。
- **Test:** 纯函数单测进 `pnpm test:ui`；行为由 V5 的 9 篇教程验收。
- **Rollback:** 分支内。
- **Depends on:** V1。
- **逼出 / Why:** 回答「Tour 在组件库里长什么样」：宿主接口怎么泛型化、教程内容用渲染函数还是插槽。

### V3 — 面板主体迁移 · 进度: 未开始

- **Fixes:** I4、I6
- **Change:**
  - `PanelApp.tsx` 拆成组合式函数（规则、配置、设置、输入模式、运行状态、按键中继）加视图组件。
  - 迁移 RuleCard、RuleGroup、HorizontalLayout、WireOverlay、KeyCapture、IntervalInput；能用组件库的一律换掉，删掉对应的自写组件与 CSS。
  - 按键录入与对话框焦点、Esc 的优先级写清楚，并配用例。
- **Acceptance:** V0 里与面板主体相关的用例在 Vue 版上全绿。
- **Test:** `pnpm test:e2e`、`pnpm test:ui`。
- **Rollback:** 分支内。
- **Depends on:** V1。

### V4 — 对话框迁移 · 进度: 未开始

- **Fixes:** I5
- **Change:**
  - 迁移 Settings、Repair、Import、ProfileCardList、About、Agreement、TourCatalog、UpdateNotice，外壳统一用组件库的 Dialog。
  - Markdown 改用 `h()` 从 `markdown-parse.ts` 的结果生成 VNode，文本一律走子节点、自动转义，链接只显示文本、完整地址放 `title`。
- **Acceptance:** V0 的对话框用例全绿；更新公告里的链接点了不导航。
- **Test:** `pnpm test:e2e`。
- **Rollback:** 分支内。
- **Depends on:** V3。

### V5 — 浮窗与教程内容 · 进度: 未开始

- **Change:** 浮窗迁移；9 篇教程内容改写到 V2 的引擎上（步骤正文从 ReactNode 换成渲染函数），GroupTimeline 迁移。
- **Acceptance:** V0 的 9 篇教程用例与浮窗用例全绿。
- **Test:** `pnpm test:e2e`。
- **Depends on:** V2、V3、V4。

### V6 — 收尾：去 React、文档、对照与真机走查 · 进度: 未开始

- **Fixes:** I8
- **Change:**
  - 删掉 React 依赖与 `@vitejs/plugin-react`；oxlint 与 oxfmt 的规则覆盖到 `.vue` 文件。
  - development 文档里的技能软链：`vercel-react-best-practices` 换掉。
  - 同步文档：architecture（前端一行与开头一句）、development（目录说明、CDP 里「等 React 重渲染」一句）、updater（Markdown 段）、testing（新增交互测试层）、`CLAUDE.md` 首行；教程路径变了的话同步 `check-skills.ts`。
  - 生成新旧并排的视觉对照页交作者确认。
  - 按 `manual/*.md` 逐篇真机走查；CHANGELOG `[Unreleased]` 记交互优化项。
- **Acceptance:** `pnpm skills:check`、`pnpm lint`、`pnpm test:ui`、`pnpm test:e2e`、`cargo clippy` 全绿；作者看过视觉对照；走查记录无退化。
- **Test:** 上述门禁 + CDP 真机走查。
- **Rollback:** 合入前分支可整体丢弃；合入后按发版回退流程。
- **Depends on:** V5。

### V7 — 回灌 Tour / Toast 到 @xwink/ui · 进度: 未开始

- **Change:**
  - 本地 `ui-pending/` 下两个组件整块搬进 xwink-console 的 `packages/ui/core`，补 owner 文档、ui-docs 示例与 `gen:ui-meta`，单测随迁。
  - xwink 侧发 beta，flair 升版本、改用包里的组件、删掉 `ui-pending/` 对应目录，并从待回灌清单移除这两条。
- **Acceptance:** flair 用包里的 Tour 跑过 V0 全部教程用例。
- **Test:** `pnpm test:e2e`；xwink 侧按它的组件库门禁。
- **Rollback:** flair 改回本地目录。
- **Depends on:** V6。组件库发 beta 属于发版，推 tag 或 publish 前一次一授。

## 5. 依赖与顺序 / Dependencies & order

```
V0 ─> V1 ─┬─> V2 ─────────┐
          └─> V3 ─> V4 ───┴─> V5 ─> V6 ─> V7
```

- **执行顺序 / Order:** V0 → V1 → V2 / V3 并行 → V4 → V5 → V6 → V7。
- **最小可交付里程碑 / Minimum valuable milestone:** 没有半途可发的子集（D1 一步到位）。V0 本身有独立价值，先做完就有了前端回归基线。
- **与其他路线图：** 搁置期间新增的 React 代码（[license](./license.md) 的激活对话框与署名输入）在重启迁移时由 V3 / V4 带过去；[pet-mode](./pet-mode.md) 开工时迁移若已重启，桌宠等 V1 之后直接用 Vue 写；[hid-stick](./archive/hid-stick.md) 后续项以后端为主，不受影响。

## 6. 工期 / Effort

口径：作者与 AI 结对的净投入天数。校准来源：[hold-interrupt](./archive/hold-interrupt.md) 第 4 节，单项界面改造（竖版统一列表、横版走线、教程改写）每卡约 1 天。本次是重写加换框架，没有旧测试兜底，整体留 +20% 余量。

| 任务卡   | 预估        | 实际 | 偏差说明                                   |
| -------- | ----------- | ---- | ------------------------------------------ |
| V0       | 2.5 天      |      | 假后端约 45 个命令，约 20 条用例，截基线图 |
| V1       | 1 天        |      | 含 `color-mix` 核实与纯 Vite 宿主缺口      |
| V2       | 2.5 天      |      | 教程引擎泛型化 + 单测，Toast               |
| V3       | 4.5 天      |      | 拆 3307 行单文件                           |
| V4       | 2 天        |      | 8 个对话框                                 |
| V5       | 1.25 天     |      | 浮窗、9 篇教程内容、GroupTimeline          |
| V6       | 1.75 天     |      | 文档、视觉对照、逐篇走查                   |
| V7       | 1 天        |      |                                            |
| **合计** | **16.5 天** |      |                                            |

最大不确定项：V3（单文件里的状态耦合有多深，拆开后才知道）、V1 的纯 Vite 宿主缺口数量。拉长日历但不占净工时：V7 等 xwink 侧发 beta。

## 7. 风险与回滚 / Risks & rollback

- **R1 迁移中途要发紧急修复。** 对策：迁移在分支上做，main 照常可发；修复同时合进分支，V0 用例补一条。
- **R2 组件库为网页设计，桌面小窗口的高密度布局要补缺口。** 对策：缺参数、缺变体走 `pnpm patch`（`apps/main/patches/`），缺整组件走 `ui-pending/`，都记进待回灌清单，按 D3 攒批次回灌。
- **R3 假后端和真后端行为不一致，测试绿了真机却挂。** 对策：V0 用 CDP 抓真实返回对齐结构；V6 的逐篇真机走查是最后一道。
- **R4 组件库版本升级带来视觉或行为变化。** 对策：锁定精确版本（与 `apps/site` 同一版本），升级单独提交并跑 `test:e2e`。

## 8. 范围外 / Out of scope · follow-ups

- 产品页 `apps/site` 已是 Vue + @xwink/ui，不在本路线图内；迁完后两者可以共享 `ui-pending/` 的组件与 Markdown 渲染。
- Mac 版的 WebView 基线（I9）：做 Mac 版时再定最低系统版本。
- 按键录入回灌组件库：网页端设置快捷键也用得上，但要先剥掉 flair 的 `KeyId` 键位定义；等 xwink 有第二个消费者时再做。
