# 中文安装包下载名称

Status: completed

## 决策

在 GitHub Actions 的 tauri-action 上传阶段使用 `气质花_[version]_[arch][setup][ext]`，例如 EXE 为 `气质花_0.7.1_x64-setup.exe`、MSI 为 `气质花_0.7.1_x64.msi`。本地 Tauri 原始构建名和应用身份保持稳定，避免因为显示名称影响覆盖升级。签名跟随安装包命名，`latest.json` 不改名。

## 验证

- 对照 tauri-action v0 的 `getAssetName`、`createArtifact`、`uploadAssets` 与 `uploadVersionJSON` 核对命名链路。
- 使用上游实际实现、合成安装包／签名和离线模拟的 GitHub 接口验证：EXE / MSI 与签名四个名称、编码下载 URL、默认 MSI 更新项、独立 NSIS / MSI 更新项及签名内容配对、固定 `latest.json` 全部通过；没有真实网络上传。
- 官网回归验证中文安装器发现、签名文件排除、编码 URL 保留、旧英文文件及草稿过滤。使用生产解析函数，新增两项测试；前端测试由提交门禁执行。
- 本轮只更新发布配置和验证，不改版本、不推送、不触发工作流或发版。下次获授权发版时验收真实 Release 资产与 `latest.json`。
