# 开发环境重复重启

Status: completed

## 原因与决策

- Tauri 日志明确记录 `src-tauri/resources/interception.dll changed` 后重启。
- `build.rs` 无条件复制该资源，内容不变也产生文件写入事件，形成重建反馈。
- 两处 DLL 目标都按内容判断是否复制，保留真实变更时的同步与资源打包。

## 任务卡

- [x] 定位触发重启的写入方。
- [x] 将 DLL 同步改为内容不变时不写入。
- [x] 验证首次复制、重复同步不改时间、同长度不同内容更新及错误传播：5 项测试通过。
- [x] 完成格式、Clippy 与文档校验。
- [x] 连续观察 45 秒：资源无写入事件，现有开发进程 PID 与启动时间未变。

## 验证边界与交接

- `cargo test -p flair-bloom --test build_resources` 使用常规资源配置，通过构建与 5 项回归。
- `cargo fmt --check`、`pnpm skills:check` 通过。
- `cargo clippy --all-targets --all-features --locked -- -D warnings` 通过。现有开发进程占用 DD DLL，因此静态检查临时通过 `TAURI_CONFIG` 将 `bundle.resources` 设为空，避免 Tauri 构建脚本覆盖使用中的资源；仓库配置保持常规资源列表。
- DLL 来源、EXE 同级副本和打包资源副本的 SHA-256 一致。未停止现有开发进程，无遗留任务。

## 工期

本轮完成，约 15 分钟。
