# 模型收藏置顶

## Goal

让用户在 chat 顶栏的模型选择器(`ModelSelector`)里"收藏"常用模型并**置顶**展示,方便快速切换,不必每次在多个 provider 分组里翻找。收藏跨会话持久化。

## 范围决定

- **只改 `src/chat/ModelSelector.tsx`**(chat 顶栏单模型选择器,换模型主入口)。
- **不动** `MultiModelSelector`(多模型并行)与设置页模型列表——本次不涉及。

## Background / 现状（已核实）

- `ModelSelector.tsx`(117 行):下拉按 provider 分组,列出 `provider.enabledModels`(空则 availableModels),项为 `providerId:model`,点击 `onModelChange(providerId, model)`。仅 `api.getSettings()` 读取,不写。
- Settings 结构体 `#[serde(rename_all="camelCase", default)]`,新增字段自动 camelCase 暴露给前端;现有类似字段 `obsidian_vault_path`(`settings.rs:1161`)。前端 `Settings` 类型在 `src/api/tauri.ts:680`,`normalizeSettings` 需补默认值。
- 保存:`save_settings` → `apply_settings`(sanitize→重应用自启/热键/托盘→persist,含回滚),**重**。收藏切换不该触发热键/托盘重注册,应走轻量专用命令(只 `persist_settings` 写盘 + 更新内存 settings)。
- 无现成 `favorite`/`pinned` 模型字段(仅会话置顶 `conversation.pinned`)。

## Requirements

- R1 Settings 新增 `favorite_models: Vec<String>`(元素为 `providerId:model`,列表顺序即置顶顺序),`#[serde(default)]`;前端 `Settings.favoriteModels?: string[]` + normalize 默认 `[]`。
- R2 新增轻量 Tauri 命令 `set_favorite_models(models: Vec<String>)`:更新内存 `state.settings.favorite_models` + `persist_settings` 写盘,**不**走 `apply_settings` 的运行时重应用。前端 `api.setFavoriteModels(models)`。
- R3 `ModelSelector` 下拉:
  - 每个模型行加星标(☆/★)按钮,点击**切换收藏**且 `stopPropagation`(不触发选中);
  - 下拉**顶部**新增"收藏"分组,按 `favoriteModels` 顺序列出已收藏模型(跨 provider),点击即切换到该模型;
  - 收藏项也带实心星标,点击可取消收藏。
- R4 只展示仍然有效的收藏(provider 存在且启用、模型在其列表中);失效的收藏在展示时过滤掉(不主动删存储,避免误删)。
- R5 切换收藏后即时持久化(调用 R2 命令),重开 app / 换会话后仍在。

## Acceptance Criteria

- [ ] AC1 点某模型行的星标 → 该模型出现在下拉顶部"收藏"组;再点 → 取消,消失。
- [ ] AC2 收藏跨 app 重启保留(写入 settings.json 的 `favoriteModels`)。
- [ ] AC3 点收藏组里的模型 → 正常切换(等价 `onModelChange`)。
- [ ] AC4 切换收藏不触发热键/托盘重注册(走轻量命令,非 `save_settings`)。
- [ ] AC5 失效收藏(provider 删了/禁用/模型没了)不在下拉里显示,也不报错。
- [ ] AC6 `cargo test`(Windows 用 `scripts/win-cargo-test.ps1`)+ `npm run lint` / `npm run typecheck` 通过;新增序列化/命令的相关测试。

## Out of Scope

- 不改 MultiModelSelector / 设置页模型列表 / ExternalModelSelector。
- 不做收藏拖拽排序(顺序 = 收藏先后;后续可加)。
- 不做"最近使用"自动置顶(与手动收藏是两回事)。

## Open Questions

（无阻塞项;范围与交互已确认。）
