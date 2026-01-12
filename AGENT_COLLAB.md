# Agent 协作记录（KeyLingo）

本文件用于多 Agent 在本仓库内协作时的“共享上下文 + 变更记录 + 交接清单”。请保持内容短、可执行；重要结论写在“决策记录”。

## 快速上下文

- 产品：KeyLingo（macOS Electron 工具：翻译 / 截图翻译 / 截图解释）。
- 架构：Electron 主进程 `electron/main.ts` + preload `electron/preload.ts` + 渲染层 React `src/`（Vite）。
- IPC 约定：渲染层只允许调用 `window.api`（已禁用透传 `ipcRenderer`）。

## 关键入口与目录

- 主进程：`electron/main.ts`
- Preload 白名单 API：`electron/preload.ts`
- 渲染层入口：`src/main.tsx`、路由/模式切换：`src/App.tsx`
- 设置页：`src/Settings.tsx`
- 截图翻译：`src/ScreenshotResult.tsx`
- 截图解释：`src/ScreenshotExplain.tsx`

## 协作规则（必须遵守）

- 不要重新引入 `window.ipcRenderer`；新增功能请扩展 `window.api` 的白名单方法。
- 不要把本机文件路径从主进程“直接暴露”给渲染层；截图解释使用 `imageId`（主进程仅允许读取自身登记的 temp 文件）。
- 外链打开走 `window.api.openExternal`；主进程侧仅允许 `https:`，如需更严格可加域名白名单。

## 常用命令

- `npm run dev`：本地开发
- `npm run build`：打包（含 `electron-builder`）
- `npm run lint`：ESLint
- `npx --no-install tsc -p tsconfig.json --noEmit`：类型检查

## 决策记录（Decision Log）

- 2025-12-31：安全加固：`ipcRenderer` 改为 `window.api` 白名单；截图解释由 `imagePath` 改为 `imageId`，并在关闭窗口时清理 temp 图片；IPC 增加 sender 校验与 `window.open` 禁用（见 `electron/main.ts`）。


## 交接模板（每次改动后追加一条）

- 时间/作者：2026-01-12 / Agent
- 改动范围（文件/功能）：
  - `electron/main.ts`：新增 OpenAI OCR 支持；重构热键注册逻辑（`registerGlobalHotkey`）；所有文件读取改为异步 (`fs.promises`)。
  - `src/Settings.tsx`：新增 OpenAI OCR 配置界面。
  - `README.md` & `package.json`：更新版本至 v1.3.5。
  - 性能优化：React 组件懒加载 (`React.lazy`)；Swift OCR 二进制缓存。
- 风险点 & 回滚方式：
  - OCR 逻辑变更较大，若截图无反应，检查控制台日志。回滚：`git revert HEAD`。
- 如何验证（最少 3 步）：
  1. 打开设置 -> 截图 -> 选择 OCR 源为 "OpenAI"，填入 Key/BaseURL，截图测试是否成功。
  2. 使用 "系统 OCR" 截图，确认速度是否提升且无卡顿。
  3. 连续开关设置/截图窗口，确认热键注册无报错 (`Command+Option+T`, `Command+Shift+A`)。
- 时间/作者：2026-01-12 / Agent
- 改动范围（文件/功能）：
  - `electron/main.ts`：更新 Store Schema 支持 `settingsLanguage`。
  - `README.md` & `package.json`：更新版本至 v1.3.6；添加多语言界面支持说明。
- 风险点 & 回滚方式：
  - 此版本仅涉及前端设置项的持久化与版本号更新，无高风险。回滚：`git revert HEAD`。
- 如何验证（最少 3 步）：
  1. 打开设置页，检查是否能切换语言（UI文本需即时更新）。
  2. 重启应用后，检查设置语言是否被记住。
  3. 确认版本号显示为 v1.3.6。
- 时间/作者：2026-01-12 / Agent
- 改动范围（文件/功能）：
  - **Cross-Platform Architecture**: Refactored `electron/main.ts` using `PlatformAdapter` pattern.
  - **Windows Support**: Implemented `win-screenshot.ps1` and C# `win-ocr` helper.
  - `electron/platforms/`: Added `interface.ts`, `mac.ts`, `windows.ts`.
  - `package.json` & `scripts/`: Added `build:win` and `build-ocr-helper.mjs` cross-platform build logic.
  - Version bump to v1.4.0.
- 风险点 & 回滚方式：
  - Windows logic is "blind coded" (verified via static analysis but not runtime). Mac logic regression tested on Mac.
  - Rollback: `git revert HEAD`.
- 如何验证（Windows）：
  - Follow `windows_verification_guide.md`.
  - Test Screenshot Translation (Ctrl+Shift+A).
  - Test AI Explanation (Ctrl+Shift+E).
- 如何验证（Mac）：
  - Existing `Command+Shift+A`, `Command+Shift+E` should work exactly as before (using `MacAdapter`).
