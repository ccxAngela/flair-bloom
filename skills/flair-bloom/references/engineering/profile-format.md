# 数据格式：配置文件、设置、协议记录、许可证

## `.qzh` 配置文件

```
┌────────────────────────────────────────┐
│ Magic       4 bytes   "QZHU"           │
│ Version     1 byte    0x01             │
│ Flags       2 bytes   reserved         │
│ Nonce      12 bytes   随机（每次写入）  │
├────────────────────────────────────────┤
│ Ciphertext  N bytes   AES-256-GCM 密文  │
├────────────────────────────────────────┤
│ Auth Tag   16 bytes   GCM 认证标签      │
└────────────────────────────────────────┘
```

- `FileHeader`（19 字节）定义在 `packages/qzh-format/src/header.rs`；`magic + version + flags` 作为 AAD，文件头篡改即验证失败。
- 密钥派生 HKDF-SHA256，输入为内嵌 32 字节应用常量（`packages/crypto/src/aes.rs` 的 `MASTER_KEY`）。它只作混淆、不保密、不轮换：首个版本起所有用户的 `.qzh` 都用它加密，换掉会让老配置与跨版本分享全部解不开；仓库公开，密钥也能从二进制取出，换了没有收益。真要轮换时用文件头 `flags` 记密钥编号并回退旧钥解密。
- 高层入口：`qzh_format::read_encrypted(path)` / `write_encrypted(path, &T)` 封装 header + aad + decrypt + parse / serialize + encrypt + atomic-rename；`qzh_profile::load_from_path` / `save_to_path` 在此之上叠加 schema 迁移、`clamp_intervals` 与业务校验。
- 宏序列文件复用同一格式，schema 由 `macro_seq.rs` 定义（`MAX_STEPS = 256`）。

## JSON schema 与迁移（策略 B）

解密后的 JSON 首字段 `schema_version` 驱动 `qzh-profile/src/schema_migrate.rs` 的迁移链（调用 `packages/migrate` 的 `run_migrations()` 泛型运行器）。读取：`version == CURRENT` 直接反序列化；`< CURRENT` 按序执行迁移函数后以新 schema 写回；`> CURRENT` 拒绝加载（`TooNew`）并提示升级应用。每个版本一个迁移函数，不允许跳版本。

**新增字段原则**：一律 `#[serde(default)]` 走向后兼容；仅字段重命名 / 移动 / 删除 / 类型变更才递增 `schema_version`。高风险版本禁止裸 bump（原因见 [release](release.md) 护栏）。

当前 `CURRENT_SCHEMA_VERSION = 4`：

| schema_version | 变更                                                                        | 引入版本 |
| -------------- | --------------------------------------------------------------------------- | -------- |
| 1              | 初始 schema                                                                 | v0.1     |
| 2              | 按键字段裸 `u32` VK → `KeyId`（键盘 + 鼠标 5 键统一），可选字段 `null` 保留 | v0.2     |
| 3              | `MouseButton` 新增 `WheelUp` / `WheelDown`                                  | v0.2.4   |
| 4              | `BurstRule` 新增可选 `group` 字段（Toggle 互斥分组）                        | v0.2.5   |

不升版本的默认值字段：`meta.signature`（水印署名，空串即未设置）。它在反序列化时就经 `sanitize_signature` 去控制字符、去首尾空白、截到 16 字符，导入、列表、保存所有读路径一处收口。`save_profile` 一律沿用文件里的旧值（前端自动保存每次从头拼 `meta`、不带署名），只有 `set_profile_signature` 能改，且要求持有 `flair.signature` 权益。

`tauri-plugin-store` 的 `settings.json` 复用同一迁移基础设施。已知键：`closeBehavior`、`holdInGroupHintDismissed`（某组变成长按与切换混排时的插队提示已勾「不再提示」）、`membershipAgreedVersion`（上次兑换时同意的会员服务协议版本，再兑换时版本没变就默认勾选）、`sound`、`theme`、`layout`、`autoUpdate`（与后端 `bootstrap/update.rs` 同名同读）、`autoEnableOnStart`（与 `bootstrap/startup.rs` 同名同读）、`tours`（`{ completed: string[], introVersion: string }`，`introVersion` 是已自动展示过的教程版本，与协议版本同一套语义）、`runAsAdmin`（「以管理员模式启动」的用户意图，注册表标志被更新抹掉时靠它自愈）、当前激活配置路径。开机自启与「以管理员模式启动」不在这里：前者由 `tauri-plugin-autostart` 写 `HKCU\Run`，后者写 `HKCU\...\AppCompatFlags\Layers`，都以注册表为准，界面只读回显。

## 输入约束

| 参数                 | 范围                                  | 执行位置                                                               |
| -------------------- | ------------------------------------- | ---------------------------------------------------------------------- |
| 连发间隔（结构下限） | 1ms – 10000ms                         | `qzh-profile/src/profile.rs::validate()`（默认 10ms）                  |
| 连发间隔（有效下限） | ≥ 10ms（`MIN_EFFECTIVE_INTERVAL_MS`） | 加载时 `Profile::clamp_intervals()` 钳位 + `burst-engine` 运行时 floor |
| 单配置规则数         | ≤ 64                                  | `profile.rs::validate()`                                               |
| 宏序列步骤数         | ≤ 256                                 | `macro_seq.rs::MAX_STEPS`                                              |

约束在 `qzh-profile` 校验层执行，不依赖前端；按键合法性不在 `validate()` 里查，见 [key-policy](key-policy.md)。

## 配置文件管理

后端命令：`save_profile` / `load_profile` / `list_profiles` / `init_default_profile` / `get_active_profile_path` / `rename_profile` / `delete_profile` / `fork_active_profile`；统一导入入口由 `commands/import_profile.rs` 提供扫描、预览与导入，支持 `.qzh`、丐帮高手、衍天高手、极简按键、小叽按键配置。`.qzh` 通过 `import_profile/qzh.rs` 直通既有解密、迁移和校验，保留完整配置（包括空规则配置），无需转换成第三方格式；所有格式复用 `save_imported_profile`，选择不冲突的名称并经 `activate_profile_file` 激活。默认配置受保护：修改默认配置自动 fork 成新配置。`.qzh` 欢迎自行研究解析与开发兼容转换工具，文件封装不构成禁止兼容研究的声明。

解析器仅保留独立实现的必要格式识别和转换规则，测试使用合成样例；不收录第三方源码、程序、真实配置、协议或授权记录。衍天默认位置为 `%APPDATA%\TheCelestialDiviner\config.json`；极简和小叽使用便携程序目录中的 INI / JSON。扫描桌面、下载及默认数据目录和一级子目录，按已知工具名只读解析快捷方式以定位程序或工作目录，不执行或 Resolve 目标程序；同一扫描流程识别 `.qzh`（扩展名不区分大小写），以规范化路径排除本应用 `profiles` 目录内的已有配置。多档位按文件路径 + `source_profile_index` 去重，预览和导入携带同一档位；直接指定文件时读取活动档位，旧衍天格式回退镜像，避免重复合并。

字典键 `K:VK:EXT` / `M:MouseInput` 对应触发键，单目标的 Hold / Toggle 转为规则，Rotate 转互斥组，保留方案和目标的启用状态。`GlobalSwitch` / `IndependentClose` 转全局启停键；双宏、多目标轮流执行、修饰键组合、小键盘扩展 Enter 等无法等价表达的动作跳过并在预览中说明。周期是原 `HoldMs + IntervalMs`，钳到本软件范围，独立按压时长无法复现。预览显示周期范围与转换警告，不将所有规则标作长按。文件大小在读取前检查 4 MB 上限，支持 UTF-8 BOM；导入仍经 `activate_profile_file`，不导入源应用的全局运行状态、驱动和界面设置。

极简按键列表迁移为默认停用的同键长按规则，并提示顺序循环无法复现；启动／停止键可迁移，NUM0～NUM9 对应源格式主键区数字。小叽按配置 ID 选择方案，按压规则保留，独立切换转互斥组，自动开关规则迁为默认停用的同键切换规则；持续按住、顺序切换、固定点点击和宏循环跳过。小叽周期计入 5 ms 按压宽度。INI 支持 UTF-16LE BOM 与 ASCII 字段，JSON 支持 UTF-8 BOM；千机魔盒无可核验样例，不注册解析器。

首装导入邀请：协议同意前读取既有 `agreed` 与 `import_welcome_seen/pending`，新装用户先持久化 pending；同意协议并完成启动状态恢复后询问扫描或跳过，选过即标 seen。老用户不会因协议／教程升级收到邀请，中途退出的新用户下次仍可选择。导入邀请、导入窗口结束前不启动教程、外设邀请或自动开启全局；主菜单也有直接入口。扫描不写源文件，确认导入后才生成新的 `.qzh`。

## 用户协议记录

启动时先检查协议：store 里 `{ agreed, agreed_at, agreement_version, app_version_at_agree }`，未同意或 `agreement_version` 与代码硬编码的 `AGREEMENT_VERSION`（`bootstrap/agreement.rs`）不一致时面板展示协议页并屏蔽其他路由；正文滚动到底才激活同意；拒绝则退出。正文 `apps/main/src/assets/EULA.md` 以 `?raw` 内联进包。条款要点：使用风险自担、仅供个人娱乐与学习、禁止商用、免责、知识产权。当前协议 v1.4。

## 许可证（billing 离线签发）

- 格式：EdDSA（Ed25519）JWS 紧凑串，与 xwink-console 的 billing 约定。头部 `alg: EdDSA`、`typ: xwink-license+jwt`、`kid`；载荷 `iss: billing`、`aud: flair`、`jti`（许可证 id）、`ent`（权益键数组）、`iat`，永久许可证不带 `exp`。`crypto::verify_license` 依次校验 `alg`、`typ`、`kid`、签名、`iss`、`aud`、时间；`typ` 把 xwink 的权益凭证（`xwink-entitlement+jwt`）挡在外面。
- 时间：`iat` 允许领先本机 24 小时，超出判时钟回拨——Windows 上时区设错让系统 UTC 偏几小时很常见，回拨本来只是顺手的一道；`exp` 到点即过期。
- 公钥：私钥只在 billing。`crypto/build.rs` 把公钥按 `kid` 编进二进制，`kid` = SPKI DER 的 SHA-256 做 base64url 取前 22 字符（与 billing 同算法），构建时由 JWK 的 `x` 重算核对，不一致即失败。
  - `FLAIR_LICENSE_PUBKEYS`：billing 交付的 JWK 数组，`release.yml` 从同名 Secret 注入。发版构建（无 `debug_assertions`）缺失、空串或 `[]` 都失败——不能发出入口还在、兑换全失败的版本；本地打不需要兑换的发版包时显式设 `FLAIR_LICENSE_ALLOW_NO_KEYS=1`，CI 不设。
  - `packages/crypto/license-dev-keys.json`：开发公钥，只在 `debug_assertions` 下编入：一把是 xwink-console 本地 billing 的开发密钥（私钥是那边入库的开发值），一把是 flair 自己的样例密钥（私钥签完 `license-dev-sample.jws` 即弃）。把样例许可证拷进 `{app_data_dir}/licenses/`（任意 `.jws` 文件名），开发构建就带上署名权益，不用起 billing。
  - 轮换：新版本加新钥，旧钥只验不签；止血时新版本移除旧钥，该钥签出的许可证全部失效、要重新激活。
- 权益：一份许可证的 `ent` 是一组权益键；许可证可叠加，本机 `{app_data_dir}/licenses/` 下一张码一个文件（文件名取归一化兑换码的 SHA-256 前 16 位，重复兑换在发请求前就能识别），可用权益取全部有效许可证的并集，同一个键取最晚到期、有永久即永久（`commands/license.rs::summarize`）。现有权益只有自定义水印署名 `flair.signature`（`crypto::ENT_SIGNATURE`），其余功能对所有用户开放；权益的说明文案与「前往」入口在前端 `panel/license.ts`。署名的存取见上文 `meta.signature`；兑换码激活尚未接入应用，进度见 `docs/roadmaps/license.md`。
