# Design — 扩展整页化

## 复用与新增
- 复用数据层：`src/settings/mcpRegistry.ts`、`src/settings/skillMarket.ts`（不动）。
- 复用后端命令：`chat_skills_install_from_url`（不动）。
- 把弹层的浏览逻辑抽成**内联浏览组件**，去掉 modal 外壳与嵌套二级弹窗。

## 组件与文件

### 1. 内联浏览组件（从 modal 抽出，去 backdrop）
- `src/chat/McpRegistryBrowser.tsx`（从 `McpMarketModal.tsx` 抽）：源切换 + 搜索 + 卡片 + 翻页 + 安装。
  needs_config 改为**卡片内联展开**（不再嵌套 modal）。props `{ existingServers, onInstall }`。
- `src/chat/SkillStoreBrowser.tsx`（从 `SkillMarketModal.tsx` ClawHub tab 抽）：排序 + 搜索 + 卡片 + 翻页 + 安装。
  props `{ onInstalled }`。
- 删除 `src/settings/McpMarketModal.tsx`、`src/settings/SkillMarketModal.tsx`。

### 2. SkillCenter.tsx 改造
- 顶部保留/微调头部；加 tab 行：`已安装 | 技能商店 | 本地导入` + 搜索框（已安装/商店各自过滤）。
- 已安装：现有卡片网格（保持）。
- 技能商店：渲染 `<SkillStoreBrowser onInstalled={refreshChatSkills}/>`。
- 本地导入：现有「导入文件夹 / 导入 zip / 打开文件夹」+ 一个 URL 输入框（调 `api.chatSkillsInstallFromUrl`）。
- 视觉尽量贴近截图（头部图标+标题+skills root 路径、状态条），但以最小改动达成，不重排既有逻辑。

### 3. McpCenter.tsx（新，整页，仿 SkillCenter 结构）
- 加载 settings（`getSettingsCached`），读 `chatTools.servers`。
- 头部：MCP 标题 + 服务器计数。tab：`已安装 | 市场 | 导入`。
- 已安装：列表行 = 名称 + transport 徽标 + 连接状态点（`api.chatMcpServerStatus` + `api.onMcpServerState`）+
  启用开关（改 `enabled` 并保存）+ 删除。仅精简管理，不做编辑表单。
  - 保存复用 SkillCenter 的"先读后端 fresh、只覆盖 chatTools 相关字段"防覆盖 OAuth token 的模式。
- 市场：`<McpRegistryBrowser existingServers={servers} onInstall={append+save}/>`。
- 导入：mcp.json（复用 `api.chatMcpImportJson` + dialog open）。

### 4. Sidebar.tsx
- `ExtensionsNavItem` 加 `'mcp'`；`extensionSubItems` 加 `{ id:'mcp', label:'MCP' }`（放在 skill 后）。

### 5. Chat.tsx
- `chatView === 'mcp'` 分支渲染 `<McpCenter onClose=.../>`（仿 skill 分支）。
- `openMcpCenter` / `handleMcpCenterClose`；`openExtensionsItem` 的 switch 加 `case 'mcp'`；
  `extensionsActive` 映射加 mcp。

### 6. SettingsShell.tsx 回退
- 删除上轮加的：`McpMarketModal`/`SkillMarketModal` import、`mcpMarketOpen`/`skillMarketOpen` state、
  「从市场安装」「技能市场」按钮、两处 modal 渲染。CLAUDE.md 的市场说明改为指向「扩展」整页。

## 关键取舍（ponytail）
- **不搬 MCP 完整编辑到 McpCenter**：精简列表 + 市场足够；详细编辑设置里已有，避免重写数百行表单/OAuth。
- **needs_config 内联展开**而非嵌套 modal：满足"无二级窗口"。
- **SkillCenter 视觉贴近但不推倒**：加 tab/搜索/卡片壳，不重构其 settings 保存与技能开关逻辑。
- MCP 连接状态订阅仅在 McpCenter 挂载期间有效（组件卸载即取消），与设置页各自独立，可接受。

## 兼容性 / 回滚
- 纯前端重构 + 删两个设置弹层；无后端/settings 结构变化。
- 回滚：恢复 SettingsShell 两个按钮、删新组件、还原 Sidebar/Chat 的 mcp 分支即可。
