# Kivio Memory System

## Goal

为 Kivio Chat 做一个 Hermes Agent 简化版记忆系统：只保留两层 Markdown 文件记忆，不引入向量库、SQLite、外部 memory provider 或语义检索。目标是让助手有一个始终在线的短小活跃记忆，以及一个不主动加载、可长期沉淀流程和经验的长期记忆。

## What I Already Know

- 用户明确不想做向量、语义库或复杂检索。
- 记忆系统分为两层：
  - L1：在线记忆、活跃记忆，始终加载进 Chat prompt。
  - L2：长期记忆、持久记忆，不主动加载，用作流程化、可复用的长期保存。
- L1 长度有限制，不能超过 5000 字节。
- L2 没有严格长度限制，但不会自动进入上下文。
- 需要新增内置工具，用于保存、修改、更新记忆。
- 设置中需要新增“记忆”选项，可启用/关闭记忆，并展示、编辑 L1/L2 内容。
- Kivio 当前 Chat 已有工具注册、上下文估算、设置页和对话文件存储，适合以文件方式实现。

## Memory Model

### L1/L2 Boundary

核心判断：

- L1 回答的是：“这个事实是否应该默认影响助手接下来每一次 Chat 回答？”
- L2 回答的是：“这个内容是否值得长期保存，但只在相关任务中按需读取？”

L1 是在线操作记忆，不是资料库；L2 是长期知识档案，不是默认上下文。

### L1 Should Store

L1 只保存短小、高频、会持续改变回答方式的信息。

适合进入 L1：

- 用户稳定偏好：语言、回答风格、是否要计划、是否要直接实现、常用格式。
- 当前活跃目标：正在做的产品方向、当前任务约束、近期明确决策。
- 需要持续遵守的硬约束：不要做向量库、不要自动加载 L2、发布必须看某文档。
- 高频项目事实：Kivio 的核心技术栈、当前工作区、关键架构边界。
- 对助手行为有直接影响的纠正：用户明确说“以后不要这样做/以后优先这样做”。
- L2 的索引式摘要：例如“长期流程在 L2 > Release Packaging 下”，而不是把完整流程放进 L1。

L1 不适合保存：

- 长流程、长代码、长日志、调研全文。
- 一次性排查细节、临时文件路径、当前对话里很快会过期的信息。
- 可以通过项目文件重新发现的完整文档内容。
- 具体历史聊天记录。
- API key、token、密码、私钥、隐私凭据。

L1 写作规范：

- 使用短 bullet。
- 每条尽量一行表达一个事实。
- 能合并就合并，避免同类事实散成多条。
- 超过 80% 容量时优先把细节归档到 L2，只在 L1 保留摘要或索引。

### L1: Online Memory

用途：

- 当前应持续影响助手回答的高价值事实。
- 用户偏好、稳定工作习惯、近期正在推进的重要上下文。
- 对 Kivio Chat 每次回答都有帮助的短句式记忆。

存储：

- `chat-memory/L1.md`
- 位于 Tauri app data 目录，不进入项目仓库。

规则：

- Chat 记忆启用时，L1 在每次 Chat 请求中自动加载。
- L1 作为独立 memory prompt segment 注入到 `build_chat_system_prompt_with_segments()`。
- L1 计入 context usage，前端展示为 Memory / L1 segment。
- L1 严格限制为 5000 字节。
- 写入、替换、整理后如果超过 5000 字节，后端拒绝保存并返回当前大小与超限原因。
- L1 内容应短、明确、可执行；不保存大段日志、临时路径、一次性聊天内容。

建议 Markdown 格式：

```md
# L1 Online Memory

- 用户偏好：回答优先用中文，直接给方案和可执行步骤。
- 项目事实：Kivio 是 Tauri v2 + React + Rust 的轻量桌面 AI 助手。
- 当前方向：记忆系统只做 L1/L2 Markdown，不做向量库。
```

### L2: Long-Term Memory

用途：

- 长期沉淀的流程、经验、复用规范、排障记录。
- 不一定每次回答都需要，但需要工具可以读取、追加、修改。
- 类似 Hermes 的长期知识库，但保持 Markdown 文件形态。

适合进入 L2：

- 可复用流程：发布流程、测试 checklist、常用排障步骤、常用命令。
- 项目长期知识：模块职责、设计决策、架构演进记录、已确认约束。
- 任务复盘：某类 bug 的原因、修复方法、以后怎么避免。
- 较长的规范：风格指南、prompt 模板、工作流说明、方案细节。
- 归档的 L1：曾经活跃、现在不必常驻，但以后可能要查的记忆。
- 按主题组织的资料：例如 Kivio、Hermes、Release、Windows、macOS 等 heading。

L2 不适合保存：

- 每轮都必须默认影响回答的核心偏好；这类应放 L1。
- 原始秘密、凭据、密钥。
- 没有复用价值的闲聊或一次性中间状态。
- 大段未经整理的日志；应保存摘要、结论和可复用步骤。

L2 写作规范：

- 用 heading 分主题。
- 内容可以比 L1 长，但仍应尽量是整理后的结论，不是原始堆积。
- 每条流程/决策尽量带日期或背景，便于未来判断是否过期。
- 如果某条 L2 内容变成近期高频约束，可以提炼一行摘要晋升到 L1。

存储：

- `chat-memory/L2.md`
- 位于 Tauri app data 目录。

规则：

- L2 不主动加载进 Chat prompt。
- L2 无严格大小限制。
- 内置工具可以读取、搜索、追加、替换、删除、将 L1 内容归档到 L2；工具数量保持最小，只暴露读和改两个入口。
- UI 可以完整展示和编辑 L2。
- 工具读取 L2 时必须有输出大小上限，避免一次性把超长文件塞回模型上下文。

建议 Markdown 格式：

```md
# L2 Long-Term Memory

## Kivio Development

### Memory System Decision

- 2026-06-06：记忆系统采用 L1/L2 Markdown 文件；L1 常驻加载，L2 仅按需读取。

## Reusable Workflows

### Release Packaging

- 发布前必须按 docs/RELEASE_PACKAGING.md 检查 DMG/MSI/NSIS 资源。
```

## Built-In Tools

新增 Kivio native tools，走现有 Chat tool registry，默认受工具审批策略控制。工具只保留两个：一个读，一个改。添加、更新、删除、归档都属于修改动作，避免工具面过散。

### Tool Visibility Rules

- L1 默认加载不是工具调用，不显示工具卡片。
- L1 默认加载属于系统提示词组装流程，和当前时间、assistant system prompt 一样是运行时上下文。
- 只有模型主动调用 `memory_read` 或 `memory_modify` 时，前端才显示工具调用。
- 读取 L1 通常不需要调用工具，因为 L1 已经默认在本轮上下文里；只有用户明确要求“看看 L1 当前内容”或模型需要确认原文时才调用 `memory_read(layer="l1")`。
- 读取 L2 必须通过 `memory_read(layer="l2")`，并显示工具调用。
- 修改 L1 或 L2 必须通过 `memory_modify`，并显示工具调用。
- 如果设置开启“工具写入确认”，`memory_modify` 在真正写盘前应走现有敏感工具确认流程。

### `memory_read`

读取记忆内容。

参数：

- `layer`: `"l1"` 或 `"l2"`
- `query`: 可选；读取 L2 时用于简单文本过滤/定位，不做语义检索
- `maxBytes`: 可选；限制返回大小

行为：

- L1 可完整读取。
- L2 默认返回匹配片段或文件开头/相关 heading，不能无限量返回。

### `memory_modify`

修改记忆内容。添加、替换、删除、归档都通过这一个工具完成。

参数：

- `layer`: `"l1"` 或 `"l2"`
- `operation`: `"append"`、`"replace"`、`"remove"` 或 `"archive"`
- `content`: append/replace 时的新 Markdown 内容
- `oldText`: replace/remove/archive 时的唯一匹配片段
- `heading`: 可选；append/archive 到 L2 时可指定目标 heading
- `archiveMode`: archive 时可选，`"move"` 或 `"copy"`，默认 `"move"`

行为：

- `append`：追加一条记忆；L2 可追加到文件末尾或指定 heading。
- `replace`：用 `content` 替换匹配到的 `oldText` 所在片段。
- `remove`：删除匹配到的片段。
- `archive`：把 L1 中的稳定内容移动或复制到 L2。
- 使用唯一子串匹配，避免要求模型知道完整文件。
- 如果匹配 0 个或多个位置，返回错误并要求更精确。
- 任何影响 L1 的修改完成后都必须校验 5000 字节限制。
- 拒绝空内容、明显凭据/API key、prompt injection 样式内容。

## Settings UI

在设置页新增“记忆”选项。

必须包含：

- 启用记忆：总开关；关闭后 L1 不注入 prompt，memory tools 不允许写入或需要明确提示不可用。
- L1 在线记忆编辑器：展示内容、保存、重置、当前字节数、5000 字节上限提示。
- L2 长期记忆编辑器：展示内容、保存、重置、当前大小提示。
- 打开记忆文件夹：方便用户备份和手动查看。
- 工具写入确认：默认开启；控制 memory tools 写入前是否需要确认。

建议包含：

- L1 注入预览：展示实际进入 prompt 的记忆块。
- 一键整理 L1：未来能力，调用模型提出压缩/归档建议；MVP 可以先不做。
- L2 搜索框：纯文本搜索 heading/关键词，不做向量检索。
- 导出/清空记忆：危险操作需二次确认。

## Prompt Injection

当记忆启用且 L1 非空时，每次用户发起 Chat 请求时加载一次 L1，并在 Chat system prompt 后追加类似：

```text
Kivio Memory (L1 online memory; user-editable; persistent across chats):

<L1.md content>
```

要求：

- 明确说明 L1 是用户可编辑的持久记忆。
- 不把 L2 自动注入。
- L1 作为单独 context segment 估算 token。
- Lens 截图问答暂不接入本记忆系统，先只作用于 Chat。
- L1 默认注入不产生 `ToolCallRecord`，不进入 assistant message 的工具调用记录。

加载流程：

1. 用户在 Chat 里发送消息或重新生成回答。
2. 后端进入 `chat_send_message` / regenerate 路径，读取 Settings。
3. 如果记忆启用，后端从 app data 的 `chat-memory/L1.md` 读取 L1 当前内容。
4. 如果 L1 为空，则不注入；如果 L1 超过 5000 字节，则请求不中断，但返回 context warning 并不注入超限内容。
5. 如果 L1 合法且非空，后端把它包装成独立的 memory prompt block，追加到本轮 Chat system prompt。
6. 本轮工具循环和最终回答共用这份 L1 快照，不在同一次工具循环中反复读文件。
7. L1 内容不写入 conversation messages，只作为本轮模型请求的运行时上下文；设置页或 memory tool 后续修改会在下一次 Chat 请求生效。

Context compression 规则：

- L1 不参与 conversation history 压缩，不应被合并进 `ConversationContextSummary` 后替代。
- 压缩上下文只处理对话历史；压缩完成后，后续每轮请求仍然重新读取 `L1.md` 并作为独立 memory block 注入。
- 如果同一次用户请求里先触发自动压缩，再继续请求模型回答，则压缩后的重建 prompt 必须重新带上同一份 L1 快照。
- 如果用户手动执行 `chat_compress_context`，压缩结果保存后下一次 Chat 请求必须照常重新注入 L1。

## Storage And File Safety

- 文件目录：`app_data_dir/chat-memory/`
- 文件：
  - `L1.md`
  - `L2.md`
- 保存方式沿用 Kivio storage 的 atomic write 风格。
- 首次启用或文件不存在时自动创建带标题的空 Markdown 文件。
- 读写必须限制在 app data 的记忆目录内，不能接受任意路径参数。
- 设置页直接编辑整份 Markdown，工具通过受限操作修改。

## Privacy And Safety

- 默认不自动记住用户对话，除非模型通过 memory tools 且用户确认。
- 不保存 API keys、token、密码、私钥、隐私凭据。
- 工具写入前做轻量安全扫描。
- L1 是每次请求都会进入模型上下文的内容，UI 必须清楚展示。
- L2 虽然不自动加载，但仍可能被工具读取；设置页应说明它是长期持久文件。

## Implementation Plan

1. 增加后端 Markdown 存储模块。
   - 创建 `chat-memory` 目录。
   - 读写 `L1.md` / `L2.md`。
   - L1 保存时校验 5000 字节。
   - 增加 Rust 单测覆盖初始化、保存、超限、子串替换。

2. 增加设置模型与 Tauri 命令。
   - Settings 增加 `chat_memory` 配置。
   - 增加读取/保存 L1/L2 的 commands。
   - 前端 `src/api/tauri.ts` / `src/chat/api.ts` 增加类型和调用。

3. 注入 L1 到 Chat prompt。
   - 在 Chat send/regenerate 前读取 L1。
   - 在 `build_chat_system_prompt_with_segments()` 追加 memory segment。
   - 更新 context usage segment。
   - 确保 context compression 后重建 prompt 时重新注入 L1。
   - 确保默认 L1 注入不显示工具调用。

4. 增加内置工具。
   - `memory_read`
   - `memory_modify`
   - 接入 native tool registry 和审批策略。
   - L2 读取和所有写入显示工具调用；默认 L1 注入不显示工具调用。

5. 设置页新增 Memory tab/section。
   - 总开关。
   - L1/L2 编辑器。
   - L1 字节计数与超限提示。
   - 保存、重置、打开文件夹。

6. 质量验证。
   - `npm run lint`
   - `npm run typecheck`
   - `cargo test --manifest-path src-tauri/Cargo.toml`
   - 手动验证设置保存、Chat prompt 注入、工具写入、L1 超限拒绝。

## Non-Goals

- 向量数据库。
- SQLite / FTS 会话索引。
- 语义检索。
- 外部 memory provider。
- 自动提取对话记忆。
- L2 自动加载。
- Lens 记忆接入。

## Open Question

L1 的 5000 限制应严格按 UTF-8 字节计算，还是按字符数计算？

推荐：严格按 UTF-8 字节计算，因为用户明确说的是“字节”，后端判断更确定；UI 同时展示“字节数”和“约等于 token 数/中文字符数”的提示，避免中文内容看起来比 5000 字少很多。
