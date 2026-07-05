# 对话分支（方案 B：分叉成新对话）

## Goal

让用户在一条已有对话的**任意一条消息处「建分支」**：把该消息及其之前的所有消息复制进一个**全新对话**，之后在新对话里独立继续，互不影响。用于「保留当前走向、另起一条探索路径」，避免现有编辑/重生成的**破坏性截断**（其后消息永久丢失）。

## Background / 现状（已核实）

- 对话是严格线性数组：`Conversation.messages: Vec<ChatMessage>`（`src-tauri/src/chat/types.rs:388`）。无父子指针、无树。
- 编辑用户提问并重生成 / 重新生成助手回复 = **破坏性截断** `messages.truncate(...)`（`commands.rs:6041 apply_regenerate_truncation`）；旧路径不可恢复。
- 已有「多模型一问多答」：`ChatMessage.group_id` + `Conversation.group_selections`，同一 user 消息 fan-out 多列答案并可选中（`MessageGroup.tsx`）。是「同节点兄弟答案」，非完整树。
- 存储：一对话一个 `conv_<id>.json`；`index.json` 为可重建缓存（`storage.rs`）。整文件原子写。
- 现成范式 `chat_import_external_conversation`（`commands.rs:483`）：新建干净对话 → 压入一串消息 → `save_message_attachments` 拷附件。分叉命令参照它。
- **附件/图片 artifact 绑定对话目录**：存 `conv_<id>_attachments/`，`Attachment.path` / artifact `path` 均为裸文件名（相对该目录，`attachments.rs`）。删对话会 `remove_dir_all` 附件目录（`storage.rs:475`）⇒ 分叉必须**深拷被引用文件**到新对话目录，否则源删除后分支损坏。
- **沙箱导出生成文件**在 `~/Kivio/outputs/<conversation_id>/`（绝对路径引用，`native_tools/sandbox_exports.rs`），按对话 id 归档。
- 新对话创建走 `create_chat_conversation_internal`（`commands.rs:339`）；前端打开对话走 `handleSelectConversation`（`Chat.tsx:2122`），入口按钮参照 `onRegenerateMessage` 透传链（`MessageList`/`MessageBubble`/`AssistantMessageMeta`/`MessageGroup`）。

## Requirements

- R1：在任意消息（user 或 assistant）上提供「分支」动作（`GitBranch` 图标，tooltip「分支」），将 `messages[0..=idx]`（含锚点）复制进一个新对话。
- R2：分叉**纯复制 + 立即打开新对话，不自动发送/重生成**；用户自行继续/编辑/重试（决策 Q1）。
- R3：新对话**继承源对话会话级配置**：provider/model、assistant 快照、project/set/folder 归属、knowledge_base_ids、thinking_level、reply_models、agent_runtime、active_skill_id。重置 pinned/context_state/agent_todo/agent_plan 为初始。
- R4：被复制消息引用的**附件与图片 artifact 文件深拷**进新对话附件目录（裸文件名同名拷贝，路径保持有效）；源对话删除不影响分支。
- R5：新对话记录来源 `forked_from`（源 id + 锚点 message id + 源标题快照）；会话顶部显示可点回跳的「分叉自 <源标题>」面包屑。
- R6：锚点落在多模型多答组的某列时，新对话**只保留该列并去掉 `group_id`**（转普通单答），丢弃其余兄弟列；同步清理指向不存在消息的 `group_selections`（决策 Q2）。
- R7：新对话标题 = `源标题 +「（分支）」`，按现有标题上限（40 字符）截断（决策 Q3）。
- R8：向后兼容——`forked_from` 为 serde default 可选字段，旧对话 JSON 缺字段正常反序列化。

## Acceptance Criteria

- [ ] AC1（R1）：在某消息建分支后，新对话包含从头到锚点（含）的全部消息，内容/角色/顺序一致；源对话完全不变。
- [ ] AC2（R4）：新对话附件与图片 artifact 可正常预览/发送；删除源对话后分支附件仍可用。
- [ ] AC3（R3/R7）：新对话继承 R3 所列配置；标题带「（分支）」后缀。
- [ ] AC4（R5）：新对话显示「分叉自 <源标题>」并可跳回源对话。
- [ ] AC5（R2）：分叉后立即在前端打开新对话，可继续输入并正常回复；不发生自动发送。
- [ ] AC6（R6）：在多答组某列建分支后，新对话该轮只有一条普通单答（无 group_id）、无残缺多答组；`group_selections` 无悬空引用。
- [ ] AC7（R8）：旧对话文件（无 `forked_from`）加载不报错。

## 已知限制（非阻断）

- 沙箱导出的**生成文件**（`~/Kivio/outputs/<源id>/`，非图片 artifact）不随分叉深拷：分支中此类 artifact 仍指向源对话 outputs 目录，源在则可用、被删则失效。附件与图片 artifact 不受此限。列为后续增强项。

## Out of Scope

- 会话内消息树 / 内联 `< 2/3 >` 切换（方案 A）。
- turn 版本化（方案 C）。
- 分叉时顺带编辑锚点或自动重生成（方案 B 的 R2 明确不做）。
