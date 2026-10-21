# Kivio Study

默认关闭的内置扩展。入口：扩展 → 管理扩展 → 启用 Kivio Study → 打开。

- `manifest.json`：包身份、API 版本及旧 `chat/reading` 地址。
- `index.tsx`：SDK 注册、页面入口和保存屏障 Hook。
- `ui/`：文献库、阅读器、笔记和配置界面。
- `api/`：本包有类型的原生命令适配器。
- `native/`：持久化、PDF、翻译、依赖安装和 Word 本机服务；`mod.rs` 对宿主提供 activate / deactivate / invoke。
- `word/`：独立 Office 任务窗格、CSL 样式和引用更新。

迁移只移动源代码，不移动用户数据。既有阅读目录、元数据、翻译版本、笔记、聊天 ID、Study 组件和 Word 设置继续使用原路径。升级后仍需显式启用。关闭前保存失败会阻止关闭，关闭后停止后台任务及 Word 端口服务；再次启用会恢复用户此前选择开启的 Word 连接，不自动安装证书或注册 Office 插件。

Word 资源继续输出到 `src-tauri/resources/word`，通过仓库 `npm run build:word` 构建。基础阅读无需配置 Python；可选 PDF2zh、Pandoc 仍在 Study 内按需安装。

## 本次迁移验收（2026-10-21）

macOS 原生开发应用重建、重启后实际操作通过：默认关闭不显示入口；手动启用进入原有两篇文献和 10 条笔记；草稿跨关闭、重启恢复；Word 本机端口随扩展关闭停止，启用并重启后恢复。用独立临时文献的目录权限制造实际写入失败，beforeDisable 阻止关闭；恢复权限、重试保存后正常关闭。临时条目已删除，原文献的笔记、草稿、聊天关联逐项比对未改变，最终保留扩展关闭状态。

自动检查：全量前端 349 文件 / 2904 项通过；Study 与扩展相关 112 项通过；既有 Rust Study 20 项通过。SDK 生命周期只新增两个用例。类型、Lint、协议、架构、前端与原生构建及桌面资源检查通过。未做 Windows 实机验收，也未在本次重新安装 PDF2zh、Pandoc 或 Office 证书。
