# Kivio Study 写作引用

用户要求：只安装 Kivio，Word 直接读取 Study；不依赖 Zotero。

## 复用结论

- [Zotero Word 插件](https://www.zotero.org/support/word_processor_plugin_usage)：成熟的动态引用流程，但由 Zotero 管理文献与文档集成，不能作为只替换数据源即可运行的独立插件。[Windows 实现](https://github.com/zotero/zotero-word-for-windows-integration)依赖 Zotero 扩展与原生 OLE 通信。
- [JabRef Word Add-in](https://github.com/JabRef/JabRef-Word-Addin)：MIT 项目，但公开接入步骤依赖它的服务端配置与开发环境。没有采用其完整应用。
- [微软引用示例](https://learn.microsoft.com/en-us/samples/officedev/office-add-in-samples/word-citation-management/)：跨平台 Office.js 接入参考，示例以 BibTeX 文件和尾注为主，不能直接满足动态 CSL 引用。本实现采用 Word 官方内容控件与文档设置 API，未复制示例代码。
- [citeproc-js](https://citeproc-js.readthedocs.io/en/latest/running.html)：实际复用的引用排版引擎。按当前文档引用顺序重建状态，更新编号、消歧与参考文献。使用上游 AGPL-3.0-or-later 许可选项；独立 Word 插件代码按相同许可分发。许可证及原始声明位于 `docs/licenses/`。
- CSL 官方样式仓库的 APA、IEEE、GB/T 7714—2015 numeric 与中英文 locale 随插件打包，保留原始作者与 CC-BY-SA 3.0 声明。

## 已实现的路径

安装正式版 Kivio → 首次启动自动注册 Word 加载项并开启本机连接 → Study → 写作引用 → 复制连接码 → Word「开始 → 加载项」中的 Kivio Study → 连接 → 搜索并选择文献 → 插入引用 / 替换当前引用 → 插入参考文献 / 刷新全文引用。

- Rust `reading/writing/word.rs` 唯一管理本机连接生命周期、证书和安装；只监听 `127.0.0.1:23127`，使用 `https://localhost:23127`。文献接口验证 Host、Origin 与 Bearer 连接码，不启用跨站 CORS；返回书目资料，不返回 PDF、笔记、聊天或密钥。
- 连接码仅复制到用户剪贴板，Word 侧只存于会话存储，绝不写入 Word 文档。关闭连接撤销当前连接码，下次开启需重新配对。
- Word `document.ts` 唯一管理文档引用。引用使用带标签的内容控件，文档设置保存已引用文献快照和引用簇；刷新依据正文控件顺序，删除引用后不再列入参考文献。资料优先使用 Study 已保存数据，已删除条目可使用文档内快照恢复。
- 跨文档复制引用尚不迁移另一文档的引用记录；无法恢复时保留原文并提示重新插入，不静默改为错误文献。只支持正文中的文内引用，不宣称脚注引用或协作编辑冲突合并。
- `.bib` 导出复用 readingLibrary 的保存等待及失败检查。固定键为 `kivio` 加去连字符的文献 UUID；标题变化不会改变键。补充卷、期、页码、出版社和 ISBN，兼容旧数据默认值，RIS 导入导出保留这些字段。

## 安装与使用边界

[macOS 官方侧载方法](https://learn.microsoft.com/en-us/office/dev/add-ins/testing/sideload-an-office-add-in-on-mac)使用用户 Word `wef` 目录。Windows 使用 [WEF Developer 注册项](https://learn.microsoft.com/en-us/office/dev/add-ins/testing/uninstall-add-in)。这是本机侧载，不是 AppSource 已发布插件。

正式版首次启动会将本机自签 HTTPS 证书加入当前用户信任，并注册加载项。macOS DMG 为拖拽安装，初始化在首次启动时完成；Windows 同样采用当前用户首次启动初始化，尚未接入 NSIS/MSI 安装器钩子。后续启动刷新注册和清单，正常升级不重复安装证书；用户关闭连接后不会被升级重新开启。开发构建不自动修改证书信任。Study 中保留安装入口用于失败重试或修复。系统可能要求用户确认；安装失败不会显示注册成功。无需额外 Node.js、Python、Zotero 或云服务。Word 的 Office.js 运行时仍使用微软官方 CDN，因此首次加载需要能访问该地址。

支持目标是 macOS / Windows 的桌面 Word，WordApi 1.3。组织策略可能禁止侧载。WPS、Word 网页版、移动 Word 本次不承诺支持。仅注册成功不代表已通过 Word 文档操作验收。

## 构建与验收

`npm run build:word` 生成加载项资源；`npm run build:ui`、`npm run dev`、`npm run package:check` 自动包含该步骤。纯 Cargo 构建前先运行 `npm run build:word`。Tauri 资源映射随桌面应用打包；Word 独立入口不加载 Tauri 启动流程。

针对性测试：真实 CSL 格式引擎验证重排、删除、重复、APA 页码、中文与 HTML 转义；Office 外部适配模拟验证重开恢复、保存失败不覆盖正文和缺失引用记录报错；真实本机 TLS HTTP 测试验证配对、来源与回收站过滤。模拟 Office 测试不能代替真实 Word。

本机未安装 Microsoft Word。正式完成验收仍需在桌面 Word 执行：安装与配对 → 插入多篇引用和页码 → 插入参考文献 → 重排 / 删除 / 切换格式并刷新 → 关闭重开 → Study 修改资料后刷新。macOS 与 Windows 安装链路分别验收。

### 本次已取得的证据

- 2026-10-20：真实 citeproc 与 Office 适配层测试 8 项；关联文献库前端测试 18 项；Rust BibTeX / HTTPS 集成测试 2 项通过。
- 全量 ESLint、TypeScript 静态检查、架构边界、打包资源及其 6 项测试通过。
- 微软 `office-addin-manifest validate` 验证清单有效。此结果不验证运行时 Word 文档操作。
- macOS 开发版 `.app` 已构建并重启；真实 Study 窗口可打开写作引用、开启连接、读取 3 篇真实书目、关闭连接并撤销连接码。
- 原生保存对话框导出 3 篇文献，Pandoc 独立读取为 CSL-JSON 成功。
- 插件面板已在 360×800 浏览器视口检查。没有在 Microsoft Word 中运行，不把浏览器渲染、API 测试或清单验证表述为 Word 验收。

- 连接初次检查失败后的重试已通过真实组件浏览器故障注入验证：失败提示与重试按钮可见，点击后恢复安装及连接操作。只替换外部状态 API，不向生产代码加入测试入口；审查唯一发现已关闭。

自动初始化的代码已接入；正式版首次启动、系统证书授权和 Windows 安装环境仍需实机验收，不以开发版验证替代。
