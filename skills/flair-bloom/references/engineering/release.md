# 发版

本项目通过 Tauri updater 向真实用户推送更新，**推 tag 触发真发布必须获得用户明确授权，一次一授**，上一次发版的授权不外推到下一次；全局约定中「演示项目免逐次授权」的项目级例外**不适用**。第 1 到 4 步代理可自主完成，第 5 步停下等指令。

## 更新日志

`CHANGELOG.md`（仓库根）是唯一内容源，格式 `## [版本号] - 日期` + 中文分节（新功能 / 问题修复 / 行为变更 / 升级方式 / 已知问题）。CI 发版时 `scripts/extract-changelog.ts` 提取当前版本节作为 GitHub Release 正文，经 updater 接口的 `update.body` 回到应用内；前端 `panel/changelog.ts` 用同一套分节规则读随包内联的这份文件，菜单里的当前版本公告与 Release 正文一致。

**填写原则**：`[Unreleased]` 记录相较上一个发布版本的**最终净变化**，不是每次提交的流水；同一功能多次迭代只写最终结果，已被撤销的改动不出现。

**权益功能**照常写进公告，但统一放在版本节最前面的 `### 亲友专属` 一节，节首用一句话说明需要兑换码，不和人人可用的改动混在一起。这样没有兑换码的用户读公告时，不会去找一个自己界面上根本没有的入口。

## 应用名称入口

改名时同步：前端 `apps/main/src/constants.ts` 的 `APP_NAME`（中文）/ `APP_NAME_EN`；Rust `apps/main/src-tauri/src/lib.rs` 的 `APP_NAME` / `APP_NAME_CN`；配置 `tauri.conf.json` 的 `productName` / `title`；`apps/main/src-tauri/Cargo.toml` 的 `name`（标识符，轻易不改）。

Windows NSIS 桌面快捷方式使用「气质花 v${VERSION}」，版本来自打包宏；产品名、EXE 名和注册表标识保持稳定。`installer.nsh` 在 PostInstall 整理静默／更新安装的已有链接，在 `.onGUIEnd` 整理完成页按用户选择创建的链接；只迁移目标为本安装目录的已知名称链接，保留其他安装目标及目标名称冲突。更新卸载保留链接，正常卸载通过 PostUninstall 清理。不要只在 PostInstall 中改名，交互安装此时尚未创建桌面链接。

公开下载的安装包名由 `release.yml` 的 `assetNamePattern: '气质花_[version]_[arch][setup][ext]'` 决定，例如 `气质花_0.7.1_x64-setup.exe` 与 `气质花_0.7.1_x64.msi`，签名追加 `.sig`。使用 tauri-action 同一命名函数上传安装包、签名并生成 `latest.json`，不手工改更新 URL。`latest.json` 名称固定；本地 Tauri 原始构建文件名沿用 CLI 默认，应用 `productName`、EXE 与安装注册表身份不因下载文件名而改变。官网按文件后缀识别安装器，不绑定英文前缀。

隔离回归：`scripts/test-installer-shortcuts.ps1 -MakeNsisPath <makensis.exe> -TauriUtilsPath <Tauri NSIS utils.nsh>`。夹具仅将桌面根目录替换为新建临时目录，实际编译并执行生产钩子，验证创建选择、完成页、升级去重、冲突保留与卸载清理，不运行正式安装器、不修改真实桌面。脚本使用 UTF-8 BOM 兼容 Windows PowerShell 5.1。

## 步骤

1. 在 `CHANGELOG.md` 的 `[Unreleased]` 填写本次内容。
2. **发版前更新文档**（强制）：逐一核对并使之与本版本实际行为一致——`README.md`（使用说明、界面示意图、「接下来」列表）、本 skill 的 `references/manual/*`（用户可见行为变了就改对应篇，并同步 `tour/tours/<id>.ts`）与 `references/engineering/*`（架构、约束、schema 版本、目录结构）、`THIRD_PARTY.md` / `EULA.md`（引入 / 变更第三方组件或权限说明时）。跑 `pnpm skills:check`。
3. `pnpm bump-version X.X.X`：自动同步三处版本号并把 `[Unreleased]` 重命名为 `[X.X.X] - 日期`（`scripts/bump-version.ts`）。
4. 提交：`chore(release): bump version to X.X.X`。
5. 获得本次发版的明确授权后，先 `git push origin main`，等待该提交的 CI 和 Coverage 门禁通过；再 `git tag vX.X.X`、`git push origin vX.X.X`。不能把 main 与 tag 一起推送，以免发布先于门禁完成。
6. tag 推送后 CI（`release.yml`：checkout → pnpm / Node 22 / Rust stable → `pnpm install` → `pnpm check:resources` → 解析上一稳定版本 tag → 提取 changelog → `tauri-action` 构建并创建 Draft Release）；核对安装包、签名、更新清单与公告后，在本次发版授权范围内发布。当前只构建 Windows x64。

## 护栏

- **高风险版本禁止裸 bump `CURRENT_SCHEMA_VERSION`**：新增字段一律 `#[serde(default)]`，仅重命名 / 移动 / 删除 / 类型变更才递增。原因：若带 schema bump 的版本出问题需「向前滚修复」，用户配置已被写成新 schema，回到旧逻辑会 `TooNew` 拒载、砸用户配置。
- **风险改动尽量挂运行时开关**：出问题先关开关止血，而不是整版回退。
- **旧 GitHub Release 不删除**：保留上个稳定版安装包 + `.sig`，确保向前滚修复与手动回退有可用且已签名的产物。

## 应急回退

`tauri-plugin-updater` 仅在远端版本高于本机时更新，永远不会降级，所以「回退」对三类用户含义不同：

| 用户群                          | 能做什么                     | 路径                |
| ------------------------------- | ---------------------------- | ------------------- |
| 还没升到坏版本                  | 拦住，别让他们升上去         | A 止血（分钟级）    |
| 已经升到坏版本                  | 无法降级，只能用更高版本盖掉 | B 向前滚修复        |
| 已静默下载暂存 `pending_update` | 召不回，下次启动必装         | 只能等 B 的新版覆盖 |

紧急回退实质 = 紧急向前滚一个修复版，不是把版本号往回拨。失败的 tag 不重指，用新版本号补发。

**路径 A：止血。** 更新端点跟着 GitHub 的 Latest 指针走，把 Latest 拨回上个好版本，`latest.json` 就不再分发坏版本。代理端点读的是同一份清单，跟随时间取决于 gh-proxy 的缓存，分钟级。

```sh
gh api repos/x-wink/flair-bloom/releases/latest --jq '.tag_name'   # 确认当前 Latest
gh release edit v0.2.7 --repo x-wink/flair-bloom --latest            # 上个好版本重新标记为 Latest
gh release edit v0.2.8 --repo x-wink/flair-bloom --draft=true        # 可选：坏版本转回 Draft，资产与页面下线
gh api repos/x-wink/flair-bloom/releases/latest --jq '.tag_name'   # 复核已切回
```

**路径 B：向前滚修复。** 发一个更高版本号的修复版，让自动更新把所有人（含已中招用户）带走。

```sh
git revert <坏提交SHA>                      # 或手动改回，提交为 fix(...)
# CHANGELOG.md 的 [Unreleased] 写明本次修复
pnpm bump-version 0.2.9
git add -A && git commit -m "chore(release): bump version to 0.2.9"
git tag v0.2.9                              # 推 tag 前一次一授
git push origin main && git push origin v0.2.9
gh run watch <run-id> --exit-status --interval 30
gh release view v0.2.9 --repo x-wink/flair-bloom --json isDraft,assets --jq '{isDraft,assets:[.assets[].name]}'
gh release edit v0.2.9 --repo x-wink/flair-bloom --draft=false --latest
```

**发布后验证速查：**

```sh
gh run view <run-id> --json status,conclusion --jq '{status,conclusion}'
gh release view vX.Y.Z --repo x-wink/flair-bloom --json isDraft,isPrerelease,assets --jq '{isDraft,isPrerelease,assets:[.assets[].name]}'
gh api repos/x-wink/flair-bloom/releases/latest --jq '.tag_name'    # updater 实际会读到的版本
tmp=$(mktemp -d); gh release download vX.Y.Z --repo x-wink/flair-bloom --pattern latest.json --dir "$tmp"; cat "$tmp/latest.json"
```

降险基建（远端最低可用版本、分批灰度、静默更新延迟安装）列在 README「接下来」，可缩小坏版本即时全量铺开的爆炸半径。
