# 准备 Kivio 发版

## Goal

准备并发布 v2.7.9 之后的 Kivio Desktop 稳定版本，确保版本元数据、双语说明、macOS/Windows 安装包、GitHub Release 与官网版本展示一致。

## Background

- 最新已发布 Tag/Release 为 `v2.7.9`，Windows NSIS 与 macOS Apple Silicon DMG 均已存在。
- 当前工作分支 `codex/fix-external-agent-issues` 包含 v2.7.9 之后的滚动修复；本地 `main` 也有尚未推送到 `origin/main` 的提交。
- Tag push 会触发 `.github/workflows/release.yml` 构建并发布 Windows NSIS；macOS DMG 必须在本机 Apple Silicon 上构建并上传。
- 版本源包括 `package.json`、`package-lock.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`、`src-tauri/tauri.conf.json`、README、官网与 release workflow 默认值。
- `website/DEPLOY.md` 与 `website/deploy.sh` 是未跟踪文件且包含明文服务器凭据，禁止纳入 Git、Release 或公开产物。

## Requirements

- R1. 选定单一版本号并同步所有版本来源。
- R2. 生成与实际变更一致的中英双语 README 摘要和 `docs/releases/vX.Y.Z.md`。
- R3. 发布前通过 lint、typecheck、前端测试、Rust 测试和本机 macOS 构建。
- R4. 发布提交必须基于将进入 `main` 的完整提交集，不能从未合并的临时分支或过期 ref 构建。
- R5. 创建 Tag 后监控 Windows workflow，上传并检查 macOS DMG，最终校正 GitHub Release 正文。
- R6. Release 必须包含 Windows NSIS 与 macOS aarch64 DMG，文件名、版本号和下载说明一致。
- R7. 网站版本展示仅在安装包/Release 可用后更新；部署凭据不得提交。

## Acceptance Criteria

- [x] AC1. 所有版本文件均为目标版本，且不存在旧版本的发布文案残留。
- [x] AC2. 发版质量门全部通过。
- [x] AC3. 发布提交已进入 `main` 并推送，目标 Tag 指向同一提交。
- [x] AC4. Windows workflow 成功，Release 同时存在 `.exe` 与 `.dmg`。
- [x] AC5. Release 正文和 README 包含双语亮点及正确 compare 链接。
- [x] AC6. 官网展示目标版本；如执行部署，线上页面可验证且不泄露凭据。

## Out of Scope

- Linux AppImage 发布。
- 将明文服务器密码提交到仓库。
- 合并未审阅的开放 PR #15 或草稿 PR #8。

## Technical Notes

- 用户已确认目标版本为 `v2.8.0`。
- 用户已确认本轮直接完成 GitHub 正式发布：推送 `main`、创建 Tag、Windows workflow、macOS DMG 与 Release 正文。
- 官网部署不在本轮执行；未跟踪的部署文件继续留在本地且不提交。
- 发版版本为 `v2.8.0`，compare 起点为 `v2.7.9`。
- 本机为 Apple Silicon，Swift 工具链可用；完整 Xcode 未配置，因此必须在创建 Tag 前验证 DMG 构建成功。
- `origin/main` 没有本地缺失的远端提交；当前分支可快进整合到 `main`。
