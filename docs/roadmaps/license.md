# 兑换码激活与水印署名 — 改造路线图 / Change Roadmap

Status: active（2026-10-28 随 v0.6.0 发版，T4 的生产真码验收待补）
Goal: 亲友在应用内输入兑换码、联网一次，换回 billing 签发的离线许可证，面板水印改显示自己配置里写的署名；之后离线校验，到期（如有）自动回落默认署名、不锁死；将来接入 xwink 登录后可把许可证认领到账号名下。

---

## 1. 背景 / Background

签发和登记交给 xwink-console 的 billing 服务（[billing 路线图](https://cnb.cool/x-wink/playground/xwink-console) `docs/roadmaps/billing.md` 的 D47、L1 / L2 卡），flair 只负责激活请求、验签和水印署名这一项权益。billing 已上线（B3 兑换码、R1 部署单元已完成），L1 激活接口未开始；格式与接口契约已与 xwink-console 对齐，见 D11–D14。

| 能力           | 现状                                                                                                                         |
| -------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| 验签           | `packages/crypto/src/license.rs` 有 Ed25519 验签与单测，但验的是自签 `QZHUA-…` 码、载荷是 `features` 位掩码，应用里无调用方 |
| 签发           | `apps/keygen` 只有空壳，仍在 workspace、CI 路径过滤与工程文档里                                                              |
| 激活接口       | billing `POST /licenses/activations`（L1），未实现                                                                           |
| HTTP 客户端    | 主应用没有直接依赖（只有 `tauri-plugin-updater` 内部用）                                                                     |
| 水印           | `.panel::after` 写死作者署名（`apps/main/src/windows/panel/PanelApp.css:75`）                                               |
| 激活面板、开关 | 未实现                                                                                                                       |
| 公钥           | 全零占位                                                                                                                     |

不变量：不用兑换码的用户行为与今天完全一致——不联网、看不到任何署名或激活相关的锁定态；`.qzh` 跨版本、跨用户照常读写。

## 2. 决策 / Decisions

- **D1：离线校验** — 理由：与「本地离线」的产品承诺一致。激活时联网见 D5。
- **D2：私钥只在 billing**，不进主应用二进制，也不进本仓。
- **D3：有到期的许可证到期后自动降级，不崩溃、不锁死** — 署名回落默认值，核心功能照常；`exp` 为空即永久。
- **D4：唯一权益是水印署名，权益键 `flair.signature`** — 作者定（2026-10-27）。宏录制回放、随机抖动、条件配置集、桌宠动画包移出本路线图，回到 README「接下来」作普通想法，是否专属以后各自立项再定；鼠标连发保持对所有用户开放。理由：署名改动面小，足以跑通激活全链路；多项权益一起做会把一条两三个会话的链路拖成几周。
- **D5：激活可以联网一次，不接受长期联网** — 作者定（2026-10-09）：像检查更新一样由用户主动触发、明确提示要联网；激活之后离线校验；不用兑换码的用户永远不联网。xwink 的权益凭证有效期只有分钟级，即使配成离线租约也要定期续期，不符合这一条，所以不用它。
- **D6：激活面板直接用现有 React 写** — 作者定（2026-10-27）：[vue-migration](./vue-migration.md) 搁置，本路线图不再等它。
- **D7：匿名激活，以后可认领** — 作者定（2026-10-09，billing D47）：
  - 激活：输入兑换码并勾选同意 xwink 会员服务协议，联网一次；billing 建一条未认领的许可证记录，返回签名的离线许可证，之后不再联网。
  - 重装或换机：同一个码在上限内可以重新下发同一许可证 id。
  - 认领：flair 接入 xwink 登录后另立路线图，凭许可证认领到账号名下。
  - 理由：要把 flair 并进 xwink 体系；只用自己的离线码、或匿名激活但不能认领，这批用户会永远游离在体系外；让亲友先注册登录，成本又不划算。
- **D8：公钥构建时注入，按 kid 维护列表** — 作者定（2026-10-09）：
  - 注入：公钥放 GitHub Secrets，`crypto` 的 build script 读环境变量生成列表。公钥不需要保密，放 Secrets 只是让公开仓里不出现它。
  - 发版构建没有公钥直接失败（缺失、空串、`[]` 都算），不回落占位；本地打不需要兑换的发版包用 `FLAIR_LICENSE_ALLOW_NO_KEYS=1` 显式豁免 — 理由：漏配时悄悄发出一个兑换码入口还在、所有激活都失败的版本，要等下一版才能修。
  - 测试公钥只在 `cfg(debug_assertions)` 下进列表，发版构建不认 — 理由：开发期要用 billing 测试端点签的许可证验证，但测试私钥的保管级别低，不能在用户手里生效。
  - 密钥隔离：每个应用一把许可证密钥，和权益凭证的密钥分开，不进 JWKS（billing D21 的例外）。flair 只认类型标记与 `aud=flair` 都对的签名，这把私钥泄露不外溢到其他服务。
  - 轮换与止血：新版本加新钥，旧钥只验不签。止血时新版本移除旧钥，代价是该钥签出的许可证全部失效，要重新激活（同码在重下发上限内可自助恢复）。
- **D9：内嵌公钥是顺手的门，不是防线** — 作者定（2026-10-09）：flair 开源免费，许可证只是亲友定制，被反编译或 fork 去掉校验影响不大；不做设备绑定，不上传硬件信息，许可证文件被拷走也接受。同理，时钟回拨只做 `iat` 下界这一道（拨到签发之前判无效，允许领先本机 24 小时：Windows 上时区设错让系统 UTC 偏几小时很常见，不放宽会刚激活就判回拨），拨回到期之前能绕过有期限的许可证，接受。
- **D10：水印署名的形态** — 作者定（2026-10-09）：
  - 字段：一整段自由文本，存在每套配置里（`meta.signature`，`#[serde(default)]`，不升 schema 版本），切配置时水印跟着换。限长 16 个字符（按 Unicode 标量计），去掉控制字符与首尾空白。反序列化即净化（`deserialize_with`），导入、列表、保存所有读路径一处收口；`save_profile` 一律沿用文件里的旧值，只有 `set_profile_signature`（要求权益）能改 — 理由：前端自动保存每次从头拼 `meta`、不带署名，不沿用旧值署名就会被清掉；`sanitize_profile` 只管按键策略，列表与导入等读路径不经过它。
  - 导出分享：导出的 `.qzh` 带上署名，导入方没有这项权益时显示默认署名，字段原样保留。
  - 入口：没有权益的用户界面上完全不露出署名入口。
  - 渲染：保留伪元素，面板根上有 `data-signature` 时用 `content: attr(data-signature)` 覆盖默认署名；没有权益或署名为空时不写这个属性，回落 CSS 里的默认值。`attr()` 取到的值按纯文本插入，不经 HTML 解析。
  - 理由：署名是亲友最直观的「专属感」，改动面小；亲友定制不对外宣传，所以不做锁定态。
- **D11：许可证格式 EdDSA（Ed25519）+ JWS 紧凑序列化** — 与 billing 对齐（2026-10-27，xwink-console-c1；最终以 billing owner 文档为准）：
  - 头部 `alg: EdDSA`、`kid`、`typ: xwink-license+jwt`。
  - 载荷 `iss: "billing"`、`aud: "flair"`（flair 在 billing 登记的应用编码，作者定 2026-10-27）、`jti`（许可证 id）、`ent`（权益键数组，如 `["flair.signature"]`）、`iat`；永久时不出现 `exp`。
  - flair 必须校验 `typ`、`aud`、`iss` 三项；权益凭证的 `typ` 是 `xwink-entitlement+jwt`，两类互不接受。
  - 理由：JWS 是通行格式，jose 与 `ed25519-dalek` 都现成支持；许可证不进 JWKS，不必跟权益凭证同用 ES256。
- **D12：`kid` 与公钥交付沿用 xwink 口径** — 与 billing 对齐（2026-10-27）：`kid` = 公钥 SPKI DER 的 SHA-256 做 base64url 后取前 22 字符（Ed25519 的 SPKI DER 是固定前缀 `302a300506032b6570032100` 加 32 字节公钥）；billing 交付 JWK `{kty:"OKP", crv:"Ed25519", x, kid}`。flair 的 build script 由 `x` 重算 `kid` 并与交付值核对，不一致即构建失败 — 理由：Secrets 里贴错一个字符时在构建期暴露，而不是在亲友激活时。
- **D13：billing 没有预发环境，开发期打本地 billing** — 与 billing 对齐（2026-10-27）：debug 构建的激活地址是 `http://localhost:8400/api/v1`（xwink-console 里 `pnpm db:up && pnpm service:all`：billing 激活前要向本地 uc 取协议版本，只起 billing 会回 503），配开发密钥，其公钥只在 debug 构建里被接受（D8）；发版构建固定打生产 `https://billing.xwink.fun/api/v1`。测试码由 billing 的本地建批次脚本产出。
- **D14：会员服务协议版本在激活时现取** — 与 billing 对齐（2026-10-27）：激活前调 uc 公开接口 `GET https://uc.xwink.fun/api/v1/auth/agreements/current?codes=membership` 取当前版本，正文页 `https://uc.xwink.fun/auth/terms/membership` 用系统浏览器打开；不在构建时写死 — 理由：激活本来就联网，现取能避免协议改版后旧版本客户端一律 `AGREEMENT_VERSION_OUTDATED`。接口返回空数组（尚无已发布协议）时不显示勾选、不传 `agreementVersion`（本地 uc 发布着草稿 `0.1.0-draft`，所以本地联调走的是「要带版本」那一支，生产目前走缺省那一支）；作者定（2026-10-27）：billing 在线上没有已发布协议时接受缺省，协议发布后必须带当前版本 — 理由：会员服务协议还在律师审，不让它卡住激活上线。
- **D15：billing L1 现在开工，由 xwink-console-c1 做** — 作者定（2026-10-27）：L1 预估 2 个会话；接口激活，不改成脚本离线签发 — 理由：亲友自助重装重下发、激活记录从第一天起记在 billing，日后要卖码时不再改链路。
- **D16：测试码只在本地，生产不建** — 作者定（2026-10-27）：开发与联调全在本地 billing；生产验收用第一张发给亲友的真码，作者在发版候选构建上先激活，亲友自己激活时算一次重下发（上限 3 内）。
- **D17：AES 主密钥不轮换，现有常量即正式密钥** — 作者定（2026-10-27）：`aes.rs` 的 `MASTER_KEY` 只作混淆、不保密，T1 删掉它的 TODO，`profile-format.md` 改写为现状 — 理由：它从首个版本起加密所有用户的 `.qzh`，换掉就让老配置与跨版本分享全部解不开；仓库公开，旧密钥已永久公开，按 D9 的逻辑新密钥也能从二进制取出，换了没有收益。将来真要轮换，用文件头 `flags` 记密钥编号并回退旧钥解密。
- **D18：激活入口放 ☰ 菜单** — 作者定（2026-10-27）：菜单加一项「兑换码」，打开激活对话框；激活后同一项打开许可证状态。
- **D19：许可证可叠加** — 作者定（2026-10-27，billing 侧 c1 确认无需改动）：一张码是一份许可证，`ent` 是一组权益（billing 每批最多 16 项）；每兑换一张码多一份许可证，可用权益取全部有效许可证按权益键的并集，同一个键取最晚到期的那份、有永久的即永久。本机 `{app_data_dir}/licenses/` 下一份许可证一个文件。理由：以后出新权益时再发新码继续往上加，已激活的不受影响，同 Steam DLC 激活码、JetBrains 按产品授权的做法。
- **D20：再兑换时协议版本没变就默认勾选** — 作者定（2026-10-27）：激活成功后把同意的会员服务协议版本记在 settings.json 的 `membershipAgreedVersion`；再兑换时 uc 现行版本与它相同就直接提交，变了才停下来要用户重新勾选。
- **D21：重复兑换先问是否覆盖，在发请求之前** — 作者定（2026-10-27）：同一张码再兑换时提示「已经激活过，覆盖会占用一次重新激活次数」，确认才提交。请求前只有码、没有许可证 id，所以许可证文件按归一化后兑换码的 SHA-256 前 16 位十六进制命名，同码同文件天然去重；覆盖即重新激活并替换该文件。理由：每次重下发都占 billing 的次数上限，发出去再问就晚了。
- **D22：兑换后用权益点说明提示怎么用** — 作者定（2026-10-27）：每项权益在前端有一段说明（是什么、在哪用）和一个「前往」入口（署名：「去填写」直达 设置 → 配置文件）；激活成功页与状态页都按权益列出。前端不认识的权益键显示键名并提示更新应用，不静默忽略。
- **D23（已被 D24 取代）：更新公告只写入口与联网，不提具体权益** — 作者定（2026-10-27）：CHANGELOG 写「☰ 菜单新增兑换码入口、权益可叠加」与「激活兑换码时联网一次」，不提水印署名。理由：入口人人可见、联网承诺变了必须告知；具体权益是亲友定制，按 D10 不对外宣传。
- **D24：权益功能写进更新公告，单独列「亲友专属」一节** — 作者定（2026-10-29），取代 D23：每项权益上线的那一版就写进公告，放在版本节最前面的 `### 亲友专属` 一节，节首说明需要兑换码；水印署名补写进 0.6.0 之后的下一版。理由：亲友拿到码后要知道能用什么、在哪里用；单独成节并明确标出来，没有兑换码的用户不会去找自己界面上没有的入口。写法见 [release](../../skills/flair-bloom/references/engineering/release.md)。

## 3. 问题清单 / Findings

| #   | 问题                                                                                                 | 严重度 | 位置                                                                                                                           | → 任务卡 |
| --- | ---------------------------------------------------------------------------------------------------- | ------ | ------------------------------------------------------------------------------------------------------------------------------ | -------- |
| I1  | 验签面向已废弃的自签码格式，公钥全零                                                                 | P1     | `packages/crypto/src/license.rs:56`                                                                                            | T1       |
| I2  | `feature_bits` 位掩码含 `MOUSE_BURST` 等五项，与 D4 冲突                                             | P2     | `packages/crypto/src/license.rs:12`                                                                                            | T1       |
| I3  | `apps/keygen` 空壳仍在 workspace、CI 路径过滤与工程文档里                                            | P2     | `Cargo.toml:12`、`.github/workflows/ci.yml:10`、`architecture.md:48`、`development.md:102`                                     | T1       |
| I4  | 工程文档写着「私钥在 keygen、公钥全零待替换」「AES 主密钥发布前替换」，与 D2 / D8 / D17 不符          | P2     | `skills/flair-bloom/references/engineering/profile-format.md:19`、`:63`                                                        | T1       |
| I5  | 水印写死作者署名，`ProfileMeta` 无署名字段                                                           | P2     | `apps/main/src/windows/panel/PanelApp.css:75`、`packages/qzh-profile/src/profile.rs:55`                                        | T2       |
| I6  | 产品承诺「除检查更新外不联网」有两处，激活上线后不再成立                                             | P1     | `README.md:148`、`apps/site/app/utils/content.ts:121`                                                                          | T3       |
| I7  | 主应用没有 HTTP 客户端                                                                               | P2     | `apps/main/src-tauri/Cargo.toml`                                                                                               | T3       |

## 4. 任务卡 / Task cards

### T1 — 验签改写与公钥注入 · 进度: 已完成（2026-10-27）

- **Fixes:** I1、I2、I3、I4
- **Change:**
  - `packages/crypto/src/license.rs` 改为验 D11 的 JWS：按 `kid` 从内嵌列表选钥，校验 `alg`、`typ`、`iss`、`aud`，再按 `iat` / `exp` 判定，返回 `jti`、`ent` 集合、`iat`、`exp`；删 `feature_bits` 与 `QZHUA` 码格式。
  - `crypto` 加 build script：读 `FLAIR_LICENSE_PUBKEYS`（JWK 数组）生成常量，按 D12 由 `x` 重算 `kid` 核对；`debug_assertions` 下追加开发公钥；发版构建缺变量或 `kid` 不符即编译失败。
  - `release.yml` 从 Secrets 注入该变量；删 `apps/keygen` 及其在 `Cargo.toml`、`ci.yml`、`architecture.md`、`development.md` 里的引用。
  - `profile-format.md` 的许可证与 AES 两段改写为现状（AES 按 D17）。
- **Acceptance:** 开发私钥签的许可证在 debug 构建验过；改 `aud`、改 `iss`、`typ` 换成 `xwink-entitlement+jwt`、未知 `kid`、篡改载荷、`iat` 晚于当前时间、已过 `exp` 各自返回对应错误；`kid` 重算与 billing 的样例 JWK 一致；`cargo build --release` 不带变量时失败、带上时通过。
- **Test:** `cargo test -p crypto`，用例在测试里现签（`ed25519-dalek` 的 `SigningKey`），不入库私钥；L1 落地后补一条本地 billing 真实签出的许可证作夹具，钉住两边编码一致。
- **Rollback:** 应用里还没有调用方，回退即还原文件。
- **Depends on:** D11 与 c1 对齐。
- **Done:** `cargo test -p crypto` 27 条全过，`license.rs` / `kid.rs` 行与函数覆盖 100%；去掉 `typ`、`aud` 校验的变异让 3 条用例变红。夹具改用 xwink-console 的 jose 加 `computeKeyId` 在 Node 里签（跨实现核对，比本地 billing 样本更早到位）；`iat` 允许领先本机 24 小时（见 D9）。发版构建实测：未设置 / 空串 / `[]` 失败，`FLAIR_LICENSE_ALLOW_NO_KEYS=1` 豁免通过，`kid` 错一位失败，正确 JWK 编入。GitHub Secret `FLAIR_LICENSE_PUBKEYS` 已填生产公钥（2026-10-27，billing 0.6.0，`kid` `fgvgx7fMl-G-xiO0AFwZZK`，已按 `x` 重算核对、实编发版构建通过）。

### T2 — 署名字段与水印渲染 · 进度: 已完成（2026-10-27）

- **Fixes:** I5
- **Change:**
  - `ProfileMeta` 加 `signature`（按 D10 反序列化即净化）；`save_profile` 经 `carry_over_meta` 沿用旧署名；新命令 `set_profile_signature` 校验权益后写入。
  - `commands/license.rs`：启动读许可证（T3 起为 `{app_data_dir}/licenses/` 目录，D19），内存只存原文、每次查询按当前时间重新验签（运行中到期自动失效）；`get_license_status` 暴露给前端。
  - 前端 `panel/signature.ts` 判定水印取值，面板根节点有权益且署名非空时写 `data-signature`，CSS `content: attr(data-signature)` 覆盖；设置 → 配置文件在有权益时多一栏「水印署名」，失焦或回车写盘，改默认配置先另存。
  - 开发公钥两把：本地 billing 的开发密钥与 flair 样例密钥；样例许可证 `packages/crypto/license-dev-sample.jws` 拷进 `{app_data_dir}/licenses/`（任意 `.jws` 文件名）即可在开发构建里试署名。
  - 说明书 `manual/profiles.md` 补署名一段，教程不动；`architecture.md` 存储路径表加许可证一行，`profile-format.md` 写字段与读写规则。
- **Acceptance:** 有权益时改署名水印立刻变、切配置跟着换；无权益时界面无任何署名入口、水印是默认值、`.qzh` 里已有的署名原样保留；导出再导入署名不丢；超长与控制字符被净化。
- **Test:** `qzh-profile` 单测覆盖净化与缺字段反序列化；前端 `pnpm test:ui` 覆盖有 / 无权益两种渲染；Windows 真机看一次横竖版水印。
- **Rollback:** 字段带 `serde(default)`、不升 schema，回退版本时旧版忽略该字段，配置不受损。
- **Depends on:** T1。
- **Done:** 单测：`qzh-profile` 净化 / 缺字段 / 反序列化截断 / 往返 4 条，主应用 `carry_over_meta` 4 条与许可证状态 4 条，crypto 补样例许可证 1 条，`pnpm test:ui` 4 条；clippy、tsc、oxlint、oxfmt 全过。**真机未验**：mac 上没有 WebView2 调试通道，横竖版水印与设置栏的实跑并入 T3 联调一起过。

### T3 — 激活链路与状态面板 · 进度: 已完成（2026-10-27）

- **Fixes:** I6、I7
- **Change:**
  - `src-tauri` 加 HTTP 客户端（复用 updater 依赖树里已有的 `reqwest`，走 rustls）与 `activate_license` 命令：按 D14 取协议版本，`POST {base}/billing/licenses/activations`，请求体 `{ code, agreementVersion? }`；成功响应 `{ licenseId, license, reissueCount, reissueLimit, expiresAt? }`（计数不含首次激活：上限 3 即激活 1 次加重下发 3 次），先验签 `license` 再落盘到 `{app_data_dir}/license.jws`，刷新权益状态。
  - 不带 `licenseId`：本地有许可证文件时再激活只会是换码，带上反而撞 `LICENSE_MISMATCH`；重装时文件已不在，同码不带它也按重下发处理。
  - 错误按响应体 `error.details.code` 映射（`error.code` 只是 `CONFLICT` 这类通用码）：`REDEMPTION_CODE_NOT_FOUND`、`LICENSE_CODE_TYPE_MISMATCH` → 码无效；`REDEMPTION_CODE_VOID` → 已作废（首次与重下发都挡）；`REDEMPTION_CODE_EXPIRED` → 已过期（只挡首次激活）；`LICENSE_EXPIRED`（410，许可证到期后再重下发）→ 许可证已到期；`LICENSE_REISSUE_LIMIT` → 重下发次数用完；`AGREEMENT_VERSION_REQUIRED` / `AGREEMENT_VERSION_OUTDATED` → 按 `details.currentVersion` 刷新协议请用户再勾一次；`LICENSE_APP_UNAVAILABLE` / `AGREEMENT_UNAVAILABLE`（503）→ 服务暂不可用；429 → 按 `Retry-After` 提示稍后再试；其余与 `LICENSE_MISMATCH` → 通用失败并记 `traceId` 到日志；连接失败 → 网络不通。billing 限流是每 IP 每分钟 5 次、每天 20 次，界面不做自动重试。
  - 激活对话框（React）：兑换码输入、会员服务协议勾选与版本显示（正文用系统浏览器打开）、「激活需要联网一次」提示、错误分类文案；状态区显示许可证 id、激活时间、到期（永久或剩余天数）、已授权的「自定义署名」；入口按 D18。
  - 许可证可叠加（D19）：`LicenseState` 存全部原文，`summarize` 按权益键合并有效期；`get_license_status` 返回 `{ entitlements, licenses }`。激活写 `licenses/<码哈希>.jws` 后重读目录，命令返回 `{ granted, summary }`，前端据此即时显示署名栏与水印，并按 D22 列出这张码带来的权益与「前往」入口。
  - 重复兑换（D21）：同码文件已存在且未带 `overwrite` 时直接返回 `already_activated`（附现有许可证），不发请求；用户确认覆盖后带 `overwrite: true` 重发。
  - 协议（D20）：激活成功后前端把同意的版本写进 settings.json，再兑换时版本没变就默认勾选。
  - 协议版本到点「激活」时才问 uc：有已发布协议时先停下让用户勾选，没有就直接激活——不点激活的人一个请求都不发。`open_link` 加 `membership` 目标打开正文页（开发构建 `http://localhost:3700/auth/terms/membership`）。
  - HTTP 用与 `tauri-plugin-updater` 同版本同 feature 的 `reqwest`（`rustls-no-provider`）与 `rustls`（`default-features = false` 只开 `ring`），加密提供者同样在首次请求前懒装；`Cargo.lock` 不新增包——rustls 默认 feature 会带进要 cmake / NASM 的 aws-lc-rs。
  - 产品承诺两处改为「除检查更新与激活兑换码外不联网」；说明书写进 `manual/profiles.md` 的「兑换码与水印署名」一节（兑换码目前只服务署名，设置篇讲的是热键声音外观）。
- **Acceptance:** debug 构建对本地 billing 用测试码激活成功、重启后仍有权益、断网启动仍有权益；同码重下发拿到同一许可证 id，第五次请求显示次数用完；作废码、类型不符、协议过期、限流、断网各显示对应文案；激活后不重启即出现署名栏，T2 的横竖版水印与设置栏一并实跑；uc 协议接口返回空数组时不显示勾选也能激活；不点激活的用户全程零网络请求（抓包或日志核）。
- **Test:** 错误码映射与落盘读回写单测（HTTP 层用本地假服务）；真机按说明书走一遍激活。
- **Rollback:** 激活入口可整体隐藏；已落盘的许可证不影响核心功能，回退版本时文件被忽略。
- **Depends on:** T1、T2、billing L1 能在本地起（生产验收要 L1 上生产）。
- **Done:** 单测：错误码映射、协议解析、许可证汇总与叠加、按码命名、重复兑换判断、目录读写共 20 条，`pnpm test:ui` 补权益说明与错误文案。真实契约：`live_activation_against_local_billing`（默认忽略，`FLAIR_TEST_LICENSE_CODE` 给码）打本地 uc / billing，验过取协议、码无效、缺协议版本带回现行版本、激活验签、同码重下发同一许可证 id。界面由作者在 mac 开发版实跑：未激活无署名栏、首次兑换停下勾协议、成功后署名栏与横竖版水印、切配置水印跟着换、错码提示、两张码叠加且第二次不再拦协议、「去填写」直达配置文件页签。重复兑换的确认框只由单测覆盖、未实跑。「不点激活零网络请求」靠代码路径保证（只有 `get_membership_agreement` / `activate_license` 两个命令联网，都在点击激活之后调用）。Windows 真机并入 T4。发版前 `/code-review`（2026-10-27，范围 `v0.5.0..`）报 10 条全部修复：验签失败的许可证先落盘再报错（时钟偏差单独提示，免得重试白占重新激活次数）、发版构建拒绝空公钥列表、默认配置另存失败时不写署名、前端按到期时间判权益且打开兑换码时重取状态、署名 `null` 容错并滤掉双向控制与零宽字符、协议接口 429 按限流处理且返回体不认识时报不可用、署名按字符截断、补 `membershipAgreedVersion` 到已知设置键。

### T4 — 生产验收与发版 · 进度: 发版完成，生产验收待补

- **Change:** billing 在生产登记 flair 应用与 `flair.signature`、建永久批次（重下发上限 3，由 c1 用脚本建，每次生产写入经作者点头）后，按 D16 用第一张真码在 Windows 发版候选构建上激活、改署名、重启；CHANGELOG `[Unreleased]` 写净变化；按 [release](../../skills/flair-bloom/references/engineering/release.md) 发版，推 tag 前取得作者授权。
- **Acceptance:** 生产真码全链路通过；发版构建的公钥列表与 billing 生产 `kid` 一致。
- **Rollback:** 新版本出问题按 release 的回退流程；billing 侧停用激活接口不影响已激活用户（D47 不可撤）。
- **Depends on:** T3、billing L1 上生产与生产批次。

## 5. 依赖与顺序 / Dependencies & order

```
T1 ─> T2 ─> T3 ─> T4
billing L1（本地可起）──┘     │
billing L1 上生产 + 批次 ─────┘
```

- **执行顺序 / Order:** T1 → T2 → T3 → T4。T1、T2 不等 L1，可立即开工；T3 的界面与错误映射也可先对着 D11–D14 的契约写，真联调等 L1 能在本地起。
- **最小可交付里程碑 / Minimum valuable milestone:** T1 + T2 可以随一个普通版本发出（无入口、无联网，用户不可见），把字段与验签先送进用户手里；T3 + T4 是对亲友可见的那一版。

## 6. 工期 / Effort

口径：代理执行、作者在场逐笔审，单位「会话」；校准来源 [hid-stick](./archive/hid-stick.md) 工期表（T1 估 1 实 0.6、T3 估 0.5 实 0.4，后端改动偏快）。

| 任务卡   | 预估      | 实际 | 偏差说明 |
| -------- | --------- | ---- | -------- |
| T1       | 0.8 会话  | ✅   |          |
| T2       | 1 会话    | ✅   |          |
| T3       | 1.5 会话  | ✅   | 中途按作者要求改为可叠加并加权益说明、协议免重复勾选、重复兑换确认（D19–D22） |
| T4       | 0.5 会话  |      |          |
| **合计** | **3.8 会话** |   |          |

不占 flair 净工时、但拉长日历的外部等待：billing L1（xwink-console 预估 2 个会话，D15 已开工）、上生产、建生产批次。

## 7. 风险与回滚 / Risks & rollback

- **R1：billing L1 进度不在本仓控制内** — T1、T2 不依赖它，先做先发；T3 对着契约写，联调推迟不返工。
- **R2：两边编码细节不一致**（base64url 填充、`ent` 形状、`exp` 缺省）— 单测只能证明自洽，L1 落地后用本地 billing 真实签出的许可证补夹具钉住。
- **R3：公钥轮换止血让旧许可证全部失效**（D8）— 亲友在重下发上限内可自助恢复。
- **R4：漏配 Secrets** — D8 让发版构建直接失败，不会发出坏版本。
- **R5：T4 验收前发版** — 作者定不等 Windows 真机复测直接发 v0.6.0（2026-10-28）：生产码还没建，没人拿得到码，兑换失败只影响兑换码入口本身，不碰核心功能；发出第一张真码前在 Windows 正式版上补验。

## 8. 范围外 / Out of scope · follow-ups

- 认领到 xwink 账号（billing L2 之后，随 flair 接入 xwink 登录另立）。
- 其他亲友专属功能（宏录制回放、随机抖动、条件配置集、桌宠动画包），见 README「接下来」。
- 到期提醒 banner：首批码都是永久的，出现有期限的批次再做。

## 9. 交接 / Handoff

- **进度（2026-10-27）：** T1–T3 完成并推送；`/code-review`（范围 `v0.5.0..`）10 条已修；GitHub Secret 已填 billing 0.6.0 的生产公钥；CHANGELOG `[Unreleased]` 已按 D23 写好。产品页 `@xwink/ui` 升到 beta.16（beta.11 被私有源的旧版本自动清理挤掉，作者已关掉该清理），Site 工作流恢复绿。
- **遗留：** 重复兑换的确认框只有单测、未实跑；Windows 上的署名水印与激活未实跑（WebView2、系统证书校验）。
- **发版（2026-10-28）：** 作者定跳过 Windows 真机复测，按 release 发 v0.6.0。
- **待排查（2026-10-29）：** 一位亲友在通用模式下兑换后切到游戏模式，billing 又记了一次激活。代码里没有自动重发的路径，第二次请求只能是手动点了「激活」或「覆盖」。两种可能：一是同一账户下误点「覆盖」；二是 Windows 标准账户提权时输入了另一个管理员的密码，`%APPDATA%` 换成那个账户的，许可证（以及规则、主题）看起来消失，于是再兑换了一次。已请她确认两点：UAC 是只点「是」还是输了密码；切换后规则和主题是否还在。必要时让她发 `%LOCALAPPDATA%\fun.xwink.flairbloom\logs\`（只保留 7 天）。若确认是第二种，要修的是「提权到另一个账户后数据目录跟着变」，影响的不只是许可证；另外日志可补记这次激活是否走了覆盖。
- **下一步入口 = T4 生产验收：** 作者定生产的 flair 码等能在 Windows 上验收时再建（向 xwink-console 会话要，`flair.signature`、永久、重下发 3 次，生产写入逐次经作者点头）→ Windows 上的 v0.6.0 正式版用第一张真码激活、填署名、重启、再兑换一次看覆盖确认；有问题按 release 向前滚修复。本地联调起法见 D13（xwink-console `pnpm db:up && pnpm service:all`），真实契约测试 `FLAIR_TEST_LICENSE_CODE=<本地码> cargo test -p flair-bloom live_activation -- --ignored`。
