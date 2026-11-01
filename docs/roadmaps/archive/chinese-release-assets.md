# 中文安装包下载名称

Status: completed

## 决策

安装包的实际下载文件名使用 ASCII `FlairBloom_[version]_[arch][setup][ext]`，中文放在 GitHub Release 资产的独立 `label`。GitHub 实际上传和重命名接口会过滤中文文件名，直接使用中文会破坏上游自动更新清单的资产查找。应用身份保持稳定，签名跟随安装包命名，`latest.json` 不改名。

## 验证

- 对照 tauri-action v0 的 `getAssetName`、`createArtifact`、`uploadAssets` 与 `uploadVersionJSON` 核对命名链路。
- 使用上游实际实现、合成安装包／签名和离线模拟的 GitHub 接口验证：EXE / MSI 与签名四个名称、编码下载 URL、默认 MSI 更新项、独立 NSIS / MSI 更新项及签名内容配对、固定 `latest.json` 全部通过；没有真实网络上传。
- 官网回归验证中文安装器发现、签名文件排除、编码 URL 保留、旧英文文件及草稿过滤。使用生产解析函数，新增两项测试；前端测试由提交门禁执行。
- 0.8.0 草稿的真实上传揭示 GitHub 对中文文件名的过滤行为，离线模拟不能覆盖这一点。发布链路增加实际资产和清单检查，避免构建成功但缺少 `latest.json`；处理与验收见 [0.8.0 发布记录](release-0.8.0.md)。
