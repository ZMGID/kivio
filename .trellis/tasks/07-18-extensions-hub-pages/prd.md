# PRD — 扩展整页化：技能商店 tab + 新 MCP 整页

## 背景
上两轮把 MCP 市场、Skill 市场做进了「设置窗口」并用了弹层/二级弹窗。用户要求改为参考
LiveAgent 的 Skills Hub / MCP Hub：**做成 Chat 主窗口「扩展」导航下的应用内整页**，
不放设置窗口、不用二级窗口（弹层）。并在「扩展」里**新增 MCP 页**。

参考 UI（LiveAgent 截图）：整页 = 头部(图标+标题+路径) + 状态条(启用开关/已选计数/扫描) +
tab 行(已安装 / 商店 / 导入) + 搜索框 + 卡片网格。

## 目标
1. **技能商店进 SkillCenter 整页**：SkillCenter 增加「已安装 / 技能商店 / 本地导入」tab + 搜索 + 卡片网格；
   技能商店 tab 内联浏览 ClawHub（复用 `skillMarket.ts`），安装走内联（needs 不弹二级窗口）。
2. **新增 MCP 整页 McpCenter**：「扩展」导航加 `MCP` 子项；整页含
   - 已安装：服务器列表（名称 + 启用开关 + 删除 + 连接状态），
   - 市场：内联注册表浏览（复用 `mcpRegistry.ts`，Official/Smithery/Glama），
   - 导入：mcp.json 导入。
   详细编辑（transport/env/headers/OAuth）仍留在设置窗口（本轮不搬）。
3. **撤掉设置窗口里的市场入口**：移除上轮加在 SettingsShell 的「从市场安装」「技能市场」按钮 + 弹层。
   保留数据层 `mcpRegistry.ts` / `skillMarket.ts` 与后端命令 `chat_skills_install_from_url`。

## 非目标
- 不把设置里的 MCP 服务器完整增删改测 / OAuth 搬到 McpCenter（精简版）。
- 不删设置窗口的 MCP / 技能 tab（详细配置仍可去那里）。
- 不引入二级窗口/模态弹层：needs_config 用卡片内联展开，不用嵌套 modal。

## 验收标准
- [ ] 左侧「扩展」出现 `MCP` 子项，点开是应用内整页（非设置、非弹层）。
- [ ] SkillCenter 呈现 已安装/技能商店/本地导入 三 tab + 搜索；技能商店能浏览/搜索/安装 ClawHub 技能；本地导入含文件夹/zip/URL 三种。
- [ ] McpCenter：已安装列表可开关/删除并显示连接状态；市场能浏览三源并安装（needs_config 内联填参）；能导入 mcp.json。
- [ ] 设置窗口不再有市场按钮/弹层；数据层与后端命令仍在。
- [ ] `npm run lint` / `npm run typecheck` / 现有+新增单测通过；`cargo` 编译通过。
- [ ] dev 手动冒烟：技能商店装 1 个、MCP 市场装 1 个（含 needs_config）、mcp.json 导入 1 次。
