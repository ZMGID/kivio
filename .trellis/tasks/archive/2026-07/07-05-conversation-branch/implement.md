# 执行计划：对话分支

## 有序清单

### 后端（Rust）
1. `src-tauri/src/chat/types.rs`
   - 新增 `ForkOrigin { conversation_id, message_id, title }`（Serialize/Deserialize/Clone/Debug/PartialEq/Eq）。
   - `Conversation` 加 `#[serde(default, skip_serializing_if = "Option::is_none")] pub forked_from: Option<ForkOrigin>`。
   - `ConversationListItem` 加同名可选字段；`From<&Conversation>` 里带上 `forked_from: conv.forked_from.clone()`。
   - 若 `Conversation`/`ConversationListItem` 有手写构造点（`create_chat_conversation_internal`、mock 等）补 `forked_from: None`。
2. `src-tauri/src/chat/commands.rs`
   - 纯函数 `fn build_fork_messages(messages: &[ChatMessage], anchor_idx: usize) -> Vec<ChatMessage>`（切片 + 多答组折叠 + 锚点去 group_id）。
   - 辅助：深拷附件/图片 artifact 文件（新私有 fn `copy_forked_conversation_files(app, src_id, new_id, messages)`；缺文件容错跳过并 eprintln warning）。
   - `#[tauri::command] chat_fork_conversation(...)`：load 源 → `find_message_index` → `build_fork_messages` → 组装新 `Conversation`（继承+重置见 design）→ 深拷文件 → 清理 `group_selections` → `save_conversation` → `strip_transcripts_for_frontend` → 返回 `{success, conversation}`。
   - 标题：`format!("{}（分支）", src.title)` 后按现有标题上限（40 字符，参照 import 流程 `.chars().take(40)`）截断。
3. `src-tauri/src/lib.rs`：`invoke_handler` 注册 `chat_fork_conversation`。

### 前端（TS/React）
4. `src/chat/types.ts`：`Conversation` 加可选 `forked_from?: { conversation_id/conversationId; message_id/messageId; title }`（与后端 snake_case 对齐，按本仓惯例兼容 camel）。
5. `src/chat/api.ts`：`forkConversation(conversationId, messageId): Promise<Conversation>`（invoke `chat_fork_conversation`；非 Tauri 走 mock 兜底）。
6. `src/chat/Chat.tsx`：`handleForkMessage(messageId)` → 调 api → `applyConversation` + ref/route + `refreshSidebar`；透传 `onForkMessage` 给 `MessageList`（见其 render props）。
7. 回调透传 + 按钮：
   - `src/chat/MessageList.tsx`：props 加 `onForkMessage`，`message`/`group` 分支透传（在飞时与 `onRegenerateMessage` 一样置 undefined 收起）。
   - `src/chat/MessageBubble.tsx`：user 气泡动作条加 `GitBranch` 按钮（`onForkMessage`）；assistant 经 `AssistantMessageMeta` 传 `onFork`。
   - `src/chat/AssistantMessageMeta.tsx`：props 加 `onFork?`，动作条加 `GitBranch` 按钮。
   - `src/chat/MessageGroup.tsx`：透传 `onForkMessage` 到每列 `MessageBubble`。
8. 面包屑：`Chat.tsx` 会话区顶部（`MessageList` 上方或其内首行）渲染 `forked_from` → 「⑂ 分叉自 <title>」按钮 → `handleSelectConversation(forked_from.conversation_id)`。

### 测试
9. Rust 单测（`commands.rs` #[cfg(test)]）：
   - `build_fork_messages` 普通前缀（含锚点、源不变）。
   - 锚点为多答组某列 → 只留该列、去 group_id、丢兄弟列。
   - `group_selections` 清理：折叠组的选中记录被淘汰、无关组保留。
10. 前端测试（vitest）：`MessageBubble.test.tsx` 补「点分支按钮调用 onForkMessage(id)」（user 与 assistant 各一）。

## 验证命令
- `npm run lint`
- `npm run typecheck`
- `npx vitest run src/chat/MessageBubble.test.tsx`
- `powershell -File scripts/win-cargo-test.ps1`（本机 Rust 测试须走此脚本，见记忆 [[windows-rust-test-manifest]]；对比 [[windows-cargo-lib-preexisting-failures]] 基线，勿把既有失败当回归）
- 手动冒烟：在有多轮（含附件、含多模型组）的对话上分支 → 新对话内容/附件正确、源不变、面包屑可跳回、可继续对话；删源对话后分支附件仍在。

## 风险文件 / 回滚点
- `types.rs`、`commands.rs`：字段/命令为纯增量，回滚只需撤销命令注册与 UI 入口。
- `MessageList.tsx`/`MessageBubble.tsx`：props 透传多，注意别破坏现有 `onRegenerateMessage` 在飞收起逻辑。

## start 前检查
- design.md 决策与本清单一致；prd Acceptance Criteria 可逐条映射到清单步骤。
