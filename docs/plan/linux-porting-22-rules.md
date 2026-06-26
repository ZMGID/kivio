# Linux 版本重构适配 22 条准则

> 本文件是 Kivio Linux 版本重构适配的前置准则。它不是僵硬的军规，而是后续所有代码修改、移植适配、打包发布前必须对齐的工程基线。

## 适配目标

- 目标系统：Ubuntu 22.04，kernel `6.8.0-40-generic`。
- 目标形态：优先验证是否能稳定产出 AppImage；若 AppImage 不适合，再进入 `.deb`、压缩包或脚本化安装方案。
- 技术栈边界：尊重现有 Tauri v2、Rust、React 18、TypeScript、Vite、TailwindCSS v4，不做无必要重写。

## 22 条准则

1. **先定基线，再改代码**：没有分析、计划、风险、进度四类 Markdown 基线前，不进入业务代码修改。
2. **先掌握主线，不泛读代码**：围绕 Linux 适配主线读取入口、构建、平台能力、打包配置；不做无目的全量通读。
3. **尊重原技术栈**：Tauri v2、Rust、React 18、TypeScript、Vite、TailwindCSS v4 是默认边界，除非证据证明无法满足 Linux 目标。
4. **AppImage 优先但不迷信**：先验证 Tauri Linux bundle/AppImage 可行性；不可行时记录失败证据，再转向替代打包。
5. **平台能力先抽象后实现**：截图、OCR、全局快捷键、窗口层级、托盘、剪贴板、自动启动等能力先定义 Linux 端口边界，再接具体实现。
6. **所有平台差异必须显式隔离**：Rust 使用 `cfg(target_os = "...")` 或平台模块隔离；前端使用平台探测接口，不散落平台判断。
7. **不得破坏 macOS/Windows 现有行为**：Linux 适配不得把既有平台从绿色路径拖入回归；必须保留跨平台回归检查。
8. **Wayland/X11 要分开建模**：屏幕级 Agent 的截图、焦点、窗口枚举、热键能力必须区分 X11、Wayland、portal/xdg-desktop-portal。
9. **屏幕捕获不可假设万能权限**：Linux 截图能力要记录桌面环境、会话类型、portal 权限与失败回退。
10. **OCR 不和截图绑死**：Linux OCR 可以走 RapidOCR、本地模型、系统组件或远程模型，但必须通过统一 OCR 端口接入。
11. **全局快捷键要有降级方案**：全局热键在不同桌面环境可能不可用；必须定义不可用时的 UI/托盘/命令触发替代路径。
12. **窗口层级要实测**：透明窗口、置顶、隐藏、skip taskbar、可见于所有工作区等能力必须在 Linux 桌面实测后再宣称支持。
13. **打包资源必须可追踪**：Pyodide、Skills、OCR 模型、外部二进制、图标、desktop 文件、AppStream 元数据都要进入打包清单。
14. **外部二进制必须跨平台解释**：当前 `kivio-ocr-helper` 是 macOS Swift sidecar；Linux 不能只靠空 stub 掩盖运行时缺口。
15. **AI Provider 协议保持稳定**：Linux 适配不得改变 OpenAI-compatible、Anthropic Messages、多 key failover 等模型协议边界。
16. **Agent 工具安全不放松**：文件工具、命令工具、MCP、Skills、子代理的权限和敏感路径保护在 Linux 上必须等价或更严格。
17. **配置优先于硬编码**：路径、依赖、模型目录、运行时资源目录、桌面环境差异必须来自配置、环境或平台解析，不写死用户机器路径。
18. **测试默认存在**：改行为、协议、平台模块、打包流程时必须补测试或验证脚本；纯文档任务要说明为何不需要测试。
19. **验证命令要有超时**：后台测试建议 60s 超时；较慢的 Rust/Tauri build 可单独声明超时边界。
20. **新增依赖要有理由**：新增 Rust crate、npm 包、系统包、AppImage 工具都要写明用途、替代方案和 Linux 发布影响。
21. **证据优先于感觉**：不能用“应该支持 Linux”替代构建日志、运行日志、包内容、桌面行为截图或命令输出。
22. **阶段确认后再执行**：每个大阶段结束必须更新进度文档，并在进入代码改动或高风险操作前取得明确确认。

## 后续代码改动入口门槛

任何 Linux 适配代码修改前，至少满足：

- `docs/analysis/project-overview.md` 已记录当前构建/打包/平台能力基线。
- `docs/analysis/risk-assessment.md` 已记录 Linux 风险与优先级。
- `docs/plan/task-breakdown.md` 已把任务拆成可验证阶段。
- `docs/progress/MASTER.md` 已标明当前阶段、下一步和验证方式。
- 用户已确认进入执行阶段。
