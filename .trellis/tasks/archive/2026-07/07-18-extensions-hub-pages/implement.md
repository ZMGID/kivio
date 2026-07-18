# Implement — 扩展整页化

## 顺序清单

### 1. 抽内联浏览组件
- [ ] `src/chat/McpRegistryBrowser.tsx`：从 `McpMarketModal` 抽浏览逻辑，去 backdrop；
      needs_config 改卡片内联展开；props `{ existingServers, onInstall }`。
- [ ] `src/chat/SkillStoreBrowser.tsx`：从 `SkillMarketModal` ClawHub tab 抽；props `{ onInstalled }`。

### 2. SkillCenter 加 tab
- [ ] 加 `view: 'installed'|'store'|'import'` 状态 + tab 行 + 搜索。
- [ ] store → `<SkillStoreBrowser>`；import → 现有导入按钮 + URL 输入；installed → 现有网格。

### 3. 新 McpCenter
- [ ] `src/chat/McpCenter.tsx`：头部 + tab（已安装/市场/导入）+ 列表(开关/删除/状态) + `<McpRegistryBrowser>` + mcp.json 导入。

### 4. 导航接线
- [ ] `Sidebar.tsx`：`ExtensionsNavItem` 加 `'mcp'` + 子项。
- [ ] `Chat.tsx`：`chatView==='mcp'` 渲染 McpCenter；open/close/switch/active 映射加 mcp。

### 5. 设置回退
- [ ] `SettingsShell.tsx`：删两个市场按钮 + 弹层 + state + import。
- [ ] 删 `src/settings/McpMarketModal.tsx`、`src/settings/SkillMarketModal.tsx`。
- [ ] `CLAUDE.md`：市场说明改为「扩展」整页（McpCenter/SkillCenter + 内联 browser）。

## 验证命令
```
npx vitest run src/settings/mcpRegistry.test.ts src/settings/skillMarket.test.ts
npx tsc --noEmit            # 0 error
npx eslint src/chat/McpRegistryBrowser.tsx src/chat/SkillStoreBrowser.tsx src/chat/McpCenter.tsx src/chat/SkillCenter.tsx src/chat/Sidebar.tsx src/chat/Chat.tsx src/settings/SettingsShell.tsx --max-warnings 0
```

## Review gates
- 组件抽好 → tsc/lint 绿再接页面。
- 全部完成 → `npm run dev` 冒烟：扩展→技能→技能商店装 1 个；扩展→MCP→市场装 1 个(含 needs_config 内联)、导入 mcp.json、列表开关/删除/状态显示。

## Rollback
- 纯前端；恢复 SettingsShell 两按钮 + 删新组件 + 还原 Sidebar/Chat 的 mcp 分支。数据层与后端命令保持。
