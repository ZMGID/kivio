# Kivio Chat — MCP 与 Skill 功能 PRD

**文档版本**: v1.1  
**创建日期**: 2026-06-03  
**产品负责人**: ZMGID  
**目标版本**: Kivio 3.1（Chat 能力增强阶段）  
**前置依赖**: Chat MVP 稳定（流式、会话持久化、独立设置 Tab 建议先完成）  
**后续阶段**: Lens ↔ Chat 联动（见桌面《Kivio AI 客户端产品需求文档 PRD》Phase 2）

---

## 一、Goal（目标）

在 **不破坏 Kivio 轻量、本地优先** 定位的前提下，为 AI Chat 增加 **MCP（Model Context Protocol）工具调用** 与 **Skill（可复用工作流指令）** 两类扩展能力，使用户在对话中即可调用外部工具、套用专业工作流，而无需切换到 Cherry Studio / Cursor 等重型客户端。

**本阶段只做 Chat**，Lens 侧复用同一套 MCP/Skill 运行时（Phase 2 接入），避免重复实现。

---

## 二、Background（背景与现状）

### 2.1 产品路线

| 阶段 | 内容 | 状态 |
|------|------|------|
| Chat MVP | 多会话、流式、附件、内嵌设置 | 进行中 |
| **本 PRD** | Chat + MCP + Skill | 待开发 |
| Lens ↔ Chat | 截图上下文转入 Chat | 明确后置 |

### 2.2 代码现状（2026-06-03）

| 模块 | 路径 | 与本次相关 |
|------|------|------------|
| Chat 前端 | `src/chat/**` | 消息 UI、InputBar、设置壳 |
| Chat 后端 | `src-tauri/src/chat/**` | 发消息、流式、本地 JSON 存储 |
| 流式 API | `src-tauri/src/api.rs` | `stream_chat_call`、`chat-stream` 事件 |
| 联网搜索（参考） | `src-tauri/src/web_search.rs`、`lens_commands.rs` | Lens 已有「规划 → 调用 → 注入上下文」模式 |
| 设置持久化 | `src-tauri/src/settings.rs` + `tauri-plugin-store` | 可扩展 `ChatConfig` |
| 前后端契约 | `src/api/tauri.ts` | 新 command / event 集中注册 |

**当前 Chat 不具备**：OpenAI `tools` / `tool_calls` 循环、MCP 客户端、Skill 加载与注入。

**Lens 已有可复用模式**：`plan_lens_web_search_tool_call` — 用模型决定是否搜索 → 执行 `search_web` → 将结果格式化为 context 再回答。Chat 的 MCP/Skill 应抽象为统一的 **Tool Runtime**，Lens 后续挂同一运行时。

### 2.3 竞品参考

| 产品 | MCP | Skill / 助手 |
|------|-----|--------------|
| Cherry Studio | MCP Server 配置、工具市场 | 300+ 预设助手（Prompt 模板） |
| Cursor | MCP + 内置 MCP 工具描述 | Agent Skills（`SKILL.md` 工作流） |
| AionUi | 统一 MCP 配置 | 内置 Agent + CLI 集成 |

**Kivio 差异化**：MCP/Skill 与 **本地对话、多 Provider、轻量 Tauri** 结合；不做 Agent 全自动改文件系统（AionUi 路线），默认 **工具调用需用户可见、可确认**。

### 2.4 多 Agent 调研结论（v1.1 补充）

本 PRD v1.1 已基于 3 个只读 explorer agent 的仓库调研结果收敛：

| 调研方向 | 关键结论 | PRD 落点 |
|----------|----------|----------|
| Chat 后端 / API | 当前 `chat_send_message` 是「保存用户消息 → 流式生成 → 保存 assistant」直线流程；`api.rs` 现有流式解析只返回文本，不保留 `tool_calls` | 新增 Chat agent loop；`api.rs` 增加可返回完整 assistant message 的 Chat Completions primitive |
| Chat 前端 / Settings | `InputBar`、顶栏、`MessageBubble` 是 Skill / Tool UI 的最小侵入接入点；当前没有专用 Chat Tools 设置页 | 新增 Chat/Tools 设置区、`SkillSelector`、assistant 内嵌 `ToolCallBlock` |
| Settings / 存储 / 权限 | Settings JSON 用 camelCase；Conversation JSON 仍是 Rust snake_case；MCP stdio 应由 Rust 后端 spawn，不暴露 frontend shell 权限 | Settings 新字段保持 camelCase；Chat 持久化新字段保持 snake_case；不新增 `shell:*` JS capability |

**v1.1 锁定的产品决策**：

- Tool call 记录持久化在对应 assistant message 的 `tool_calls` 元数据中，不作为独立 timeline message。
- Skill 是 **会话固定、可切换**：选中后绑定当前 conversation，后续消息默认沿用，可在顶栏/输入栏清除或切换。
- Tool 默认确认策略为 **读类自动、敏感确认**：搜索、fetch、只读查询自动执行；写文件、删文件、执行命令等敏感工具调用前弹窗确认。
- MCP 默认不启用；导入 server 后也必须用户显式启用。
- Native `web_search` 不算 MCP Server，但在 UI、事件、agent loop 中与 MCP Tool 统一展示。

---

## 三、Definitions（概念定义）

### 3.1 MCP（Model Context Protocol）

在本产品中，MCP 指：Kivio **作为 MCP Client**，连接用户配置的一个或多个 **MCP Server**，将其暴露的 **Tools** 转为模型可用的 function calling 接口。Resources / Prompts 不进入 MVP。

**MVP 支持**：
- Transport：**stdio**（子进程，与 Cursor `mcp.json` 兼容）
- 可选 Phase 3：**Streamable HTTP** 远程 Server（旧 HTTP+SSE Server 仅作为兼容方向评估）

**典型 Server 示例**：filesystem、fetch、sqlite、brave-search、自定义脚本。

### 3.2 Skill

在本产品中，Skill 指：一份 **结构化的 Markdown 工作流说明**（兼容 Cursor `SKILL.md`  frontmatter 约定），用于：

1. **注入系统层指令**（领域知识、步骤、输出格式）
2. **声明推荐启用的 MCP 工具子集**（可选）
3. **声明触发场景**（description 字段供 UI 展示与搜索）

Skill **不是**可执行代码；执行仍由模型 + MCP Tools 完成。

**示例 Skill**：「技术文档润色」「Git commit message 生成」「论文摘要结构化」。

### 3.3 MCP vs Skill 关系

```
用户选择 Skill（工作流/人设）
        ↓
系统 Prompt += Skill 正文
        ↓
用户提问 → 模型决定是否调用 MCP Tool
        ↓
Tool Runtime 执行 MCP Server → 结果回注 → 继续流式回答
```

---

## 四、User Stories（用户故事）

1. **作为用户**，我可以在 Chat 设置里添加 MCP Server（命令、参数、环境变量），并测试连接是否成功。
2. **作为用户**，我可以在对话输入栏旁 **选择/切换 Skill**，让 AI 按特定工作流回答。
3. **作为用户**，当 AI 调用 MCP 工具时，我能 **看到调用了什么工具、参数摘要、执行状态**，而不是黑盒。
4. **作为用户**，对于敏感工具（写文件、执行命令），我可以在设置里要求 **每次调用前确认**。
5. **作为用户**，我可以从文件夹 **导入 Skill**（`SKILL.md`），并在对话中复用。
6. **作为开发者/高级用户**，我可以使用与 Cursor 类似的 `mcp.json` 格式，减少迁移成本。

---

## 五、Functional Requirements（功能需求）

### 5.1 MCP 管理（Settings）

| ID | 功能 | 说明 | 优先级 |
|----|------|------|--------|
| M1 | MCP Server 列表 | 增删改查：name、command、args、env、cwd、enabled | P0 |
| M2 | 连接测试 | `chat_mcp_test_server`：list tools，展示工具名与 description | P0 |
| M3 | 全局启用开关 | `chat.mcpEnabled`，关闭后不向模型暴露任何 MCP tools | P0 |
| M4 | 导入 mcp.json | 从文件导入（Cursor 兼容 schema 子集） | P1 |
| M5 | 按 Server 启用 Tool | 细粒度勾选可用 tools（防止工具过多撑爆 context） | P1 |
| M6 | SSE/HTTP Transport | 远程 MCP Server | P2 |

**配置存储**：`settings.json` 内 `chat.mcpServers[]`；密钥类 env 随现有 settings 明文策略（与 apiKeys 一致），文档中提醒用户风险。

**默认内置 MCP（可选 P1）**：
- 复用现有 **`web_search`**（Tavily/Exa）作为 **Native Tool**，不走 MCP 进程，但与 MCP Tool 在 UI 和 agent loop 中统一展示。

### 5.2 Skill 管理

| ID | 功能 | 说明 | 优先级 |
|----|------|------|--------|
| S1 | Skill 库列表 | 扫描目录展示：内置 + 用户目录 | P0 |
| S2 | Skill 目录 | 默认 `{app_data}/skills/`；可选额外扫描路径 | P0 |
| S3 | 导入 Skill | 选择文件夹或 zip（含 `SKILL.md`） | P1 |
| S4 | 对话级 Skill 选择 | InputBar 或顶栏：当前 Skill（可为空） | P0 |
| S5 | Skill 预览 | 侧栏/弹窗只读展示 Skill 正文 | P1 |
| S6 | 内置 Skill 包 | 随应用附带 3–5 个通用 Skill（翻译润色、代码解释、会议纪要） | P1 |
| S7 | Skill 关联 MCP | Skill frontmatter `mcp-tools: [tool-a, tool-b]` 限制工具集 | P2 |

**Skill 文件格式（兼容 Cursor）**：

```markdown
---
name: tech-doc-polish
description: 润色技术文档，统一术语与 Markdown 结构。用户提到文档、README、技术写作时使用。
recommended-tools:
  - fetch
---

# 技术文档润色

（工作流正文：步骤、约束、输出模板…）
```

解析规则：`name`、`description` 必填；`recommended-tools` 可选。

### 5.3 Chat 对话内 Tool 调用

| ID | 功能 | 说明 | 优先级 |
|----|------|------|--------|
| T1 | Agent Tool Loop | 发消息 → 模型返回 tool_calls → 执行 → 结果 messages 回注 → 直至无 tool_calls | P0 |
| T2 | 最大轮次 | 默认 `maxToolRounds: 5`，可配置，防止死循环 | P0 |
| T3 | 流式兼容 | 工具执行阶段 emit 进度；最终回答仍走 `chat-stream` | P0 |
| T4 | Tool Call UI | 消息流中折叠块：工具名、状态、耗时、结果摘要（参考 `WebSearchBlock`） | P0 |
| T5 | 用户确认 | 敏感 tool 分类（写/删/执行）弹窗确认 | P1 |
| T6 | 取消 | 工具执行中可取消整轮生成（复用 `explain_stream_generation`） | P0 |
| T7 | 持久化 | assistant 消息存 `toolCalls[]` 元数据，刷新后可还原 UI | P1 |

### 5.4 Provider 兼容性

| ID | 要求 | 优先级 |
|----|------|--------|
| P1 | 支持 OpenAI 兼容 `tools` + `tool_calls` 的 Provider | P0 |
| P2 | 不支持 tools 的 Provider：Skill 仍可用（仅 Prompt 注入）；MCP 入口 disabled + 设置内提示 | P0 |
| P3 | 推理模型：tool call 与 `reasoning_content` 并存时，UI 分区展示 | P1 |

---

## 六、Non-Functional Requirements（非功能需求）

| 类别 | 要求 |
|------|------|
| 性能 | 单个 MCP Server 冷启动 < 3s；Tool list 缓存 5min |
| 安全 | MCP 子进程无 Tauri webview 权限；默认不启用高危 Server；确认框可配置 |
| 隐私 | Tool 输入输出仅存本地对话 JSON；不上传 телемetry |
| 稳定 | 单个 Server crash 不影响其他 Server；超时默认 60s（与 HTTP client 一致） |
| 体积 | 优先 Rust MCP 客户端库或自研 stdio JSON-RPC；避免捆绑 Node runtime |
| 兼容 | macOS 14+、Windows 10/11；Linux 不在本阶段范围 |

---

## 七、UX 设计要点

### 7.1 设置页 — Chat → 工具与 Skill

```
Chat 设置
├── 模型与行为（已有/规划中）
├── MCP 服务器
│   ├── [启用 MCP] Toggle
│   ├── Server 列表（名称、状态、工具数）
│   ├── [添加服务器] [从 mcp.json 导入]
│   └── 敏感操作确认 Toggle
└── Skill 库
    ├── 扫描路径
    ├── Skill 列表（名称、描述、来源）
    └── [导入 Skill 文件夹]
```

### 7.2 对话界面

```
┌─ 侧栏 ─┬─ 顶栏：模型选择 | Skill: [未选择 ▼] ─────────────┐
│        ├─ 消息区                                          │
│        │   [User] ...                                       │
│        │   [Tool] 🔧 fetch · 已完成 · 1.2s  [展开]          │
│        │   [Assistant] ...                                  │
│        └─ InputBar [+] [Skill] [MCP 指示器] [发送] ─────────┘
```

- **Skill 选择器**：空 = 无额外工作流；选中后顶栏显示 Skill 名，可一键清除。
- **MCP 指示器**：显示当前会话已启用工具数；点击跳转设置。
- **Tool 块**：视觉对齐 `src/lens/WebSearchBlock.tsx`（状态 icon、可折叠详情）。

### 7.3 空状态与错误

- 未配置 MCP：InputBar 不显示工具相关 UI；设置中有引导链接。
- Server 连接失败：设置页红色状态 + 最近错误信息。
- Model 不支持 tools：发送前 toast 提示，Skill 仍生效。

---

## 八、Technical Architecture（技术方案）

### 8.1 模块划分

```
src-tauri/src/
├── chat/
│   ├── commands.rs          # 扩展 send_message → agent loop
│   ├── storage.rs
│   └── types.rs             # ToolCallRecord, SkillMeta
├── mcp/                     # 新增
│   ├── mod.rs
│   ├── client.rs            # stdio JSON-RPC session
│   ├── registry.rs          # server 生命周期、tool 聚合
│   └── types.rs
├── skills/                  # 新增
│   ├── mod.rs
│   ├── loader.rs            # 扫描 SKILL.md、解析 frontmatter
│   └── types.rs
└── agent/                   # 新增（或 chat/agent_loop.rs）
    ├── mod.rs
    ├── loop.rs              # tool round-trip
    └── native_tools.rs      # web_search 等内置工具
```

```
src/
├── chat/
│   ├── ToolCallBlock.tsx    # 新组件
│   ├── SkillSelector.tsx    # 新组件
│   └── ...
├── settings/
│   ├── ChatMcpSettings.tsx  # 新组件
│   └── ChatSkillSettings.tsx
└── api/tauri.ts             # 新 invoke / event 类型
```

### 8.2 Agent Loop 伪代码

```rust
async fn chat_agent_complete(conversation, skill, mcp_registry, native_tools) {
    let mut messages = build_messages(conversation, skill);
    let tools = merge_tool_schemas(mcp_registry, native_tools, skill_filter);

    for round in 0..max_tool_rounds {
        let response = stream_chat_with_tools(messages, tools).await?;

        if response.tool_calls.is_empty() {
            return finalize_assistant_message(response.content);
        }

        emit_chat_tool_event("running", &response.tool_calls);

        for call in response.tool_calls {
            maybe_confirm(call)?; // 敏感工具
            let result = execute_tool(call, mcp_registry, native_tools).await?;
            messages.push(tool_result_message(call.id, result));
        }

        emit_chat_tool_event("done", ...);
    }
    Err("Max tool rounds exceeded")
}
```

### 8.3 数据模型扩展

**Settings — `ChatConfig` 扩展**：

```typescript
interface ChatMcpServer {
  id: string
  name: string
  enabled: boolean
  transport: 'stdio' // | 'streamable_http' later
  command: string
  args: string[]
  env: Record<string, string>
  cwd?: string
  enabledTools?: string[] // 空 = 全部
}

interface ChatToolsConfig {
  enabled: boolean
  servers: ChatMcpServer[]
  skillScanPaths: string[]
  maxToolRounds: number // default 5, clamp 1-10
  toolTimeoutMs: number // default 60000
  maxToolOutputChars: number // default 12000
  approvalPolicy: 'readonly_auto_sensitive_confirm'
  nativeTools: {
    webSearch: boolean
  }
}

interface ChatConfig {
  // ...existing chat fields...
  chatTools: ChatToolsConfig
}
```

**Message 扩展**：

```typescript
interface ToolCallRecord {
  id: string
  name: string
  serverId?: string
  arguments: string // JSON string, truncated in UI if huge
  status: 'pending' | 'running' | 'success' | 'error' | 'cancelled'
  resultPreview?: string
  error?: string
  durationMs?: number
}

interface ChatMessage {
  // ...existing...
  toolCalls?: ToolCallRecord[] // frontend camelCase; persisted Rust field is tool_calls
  activeSkillId?: string // conversation-pinned Skill snapshot for this assistant response
}

interface Conversation {
  // ...existing...
  activeSkillId?: string // frontend camelCase; persisted Rust field is active_skill_id
}

// 注意：Settings serde 继续使用 camelCase；Chat conversation JSON 继续保持 snake_case，
// 新增 Rust 字段必须 #[serde(default)]，避免旧 conv_*.json 反序列化失败。
```

### 8.4 新增 Tauri Commands & Events

| Command | 说明 |
|---------|------|
| `chat_mcp_list_tools` | 列出所有已启用 server 的 tools |
| `chat_mcp_test_server` | 测试单个 server |
| `chat_skills_list` | 扫描并返回 Skill 元数据 |
| `chat_skills_read` | 读取 Skill 正文 |
| `chat_send_message` | **扩展**：支持 `activeSkillId?` / run options；内部走 agent loop |

| Event | Payload | 说明 |
|-------|---------|------|
| `chat-stream` | `{ conversationId, runId, messageId?, kind, delta, reasoningDelta?, done?, reason?, full? }` | 必须带稳定 run/conversation 关联；兼容现有 answer/reasoning 展示 |
| `chat-tool` | `{ conversationId, runId, messageId?, toolCallId, name, source, serverId?, status, argumentsPreview?, resultPreview?, error?, startedAt?, completedAt?, durationMs? }` | 工具进度与持久化同步 |
| `chat-tool-confirm` | `{ conversationId, runId, toolCallId, name, source, serverId?, argumentsPreview, sensitivity }` | 敏感工具调用前确认 |

前端 **`src/api/tauri.ts`** 为唯一契约源。Settings serde 使用 `camelCase`；Chat conversation 持久化沿用当前 snake_case，不做全量 schema 迁移。

### 8.5 依赖选型（待 Spike）

| 方案 | 说明 |
|------|------|
| A. Rust crate `rmcp` / 社区 MCP SDK | 优先调研，减少自研协议 |
| B. 自研 stdio JSON-RPC | MVP 仅 `tools/list` + `tools/call`，控制体积 |
| C. Sidecar Node MCP | 体积大，**不推荐** |

**Spike 任务（Phase 0，1–2 天）**：用 1 个 stdio MCP Server（如 `@modelcontextprotocol/server-fetch`）验证 list/call 与 Tokio 子进程管理。

### 8.6 与 Lens 的统一（Phase 2 预留）

- 抽出 `agent/` 为 feature 级模块；Lens `lens_ask` 可选走同一 `agent_loop`。
- `web_search` 从 Lens 专有逻辑迁入 `native_tools.rs`。
- 本阶段 **不修改 Lens UI**，仅保证架构可复用。

---

## 九、Phases & Milestones（分期交付）

### Phase 0 — Spike & 基础（3–5 天）

- [ ] MCP stdio 连接 POC（list tools / call tool）
- [ ] Skill loader POC（解析 frontmatter + body）
- [ ] 确定 `ChatConfig` schema 与 migration
- [ ] 更新 `.trellis/spec` 跨层契约文档

### Phase 1 — MVP（2–3 周）

**目标**：Chat 内可配置 1 个 MCP Server + 选择 Skill + 可见 Tool 调用。

- [ ] Settings：MCP Server CRUD + 测试连接
- [ ] Settings：Skill 列表（内置 + 用户目录）
- [ ] Chat：Skill 选择器 + system prompt 注入
- [ ] Chat：Agent loop + `chat-tool` 事件 + `ToolCallBlock` UI
- [ ] Native tool：`web_search`（复用 `web_search.rs`）与 MCP tool 统一
- [ ] 停止生成 / 取消 tool 执行
- [ ] Provider 不支持 tools 时的降级

**验收标准**：
1. 配置 fetch MCP 后，问「抓取 example.com 标题」能触发 tool 并返回答案
2. 选择「代码解释」Skill 后，回答格式符合 Skill 要求
3. Tool 调用过程在 UI 可见；对话刷新后仍可查看记录
4. `npm run lint` / `typecheck` / `cargo test` 通过

### Phase 2 — 体验与安全（1–2 周）

- [ ] 敏感工具确认框
- [ ] mcp.json 导入
- [ ] 按 Server 筛选 tools
- [ ] Skill 导入 + 预览
- [ ] 3–5 个内置 Skill
- [ ] Tool 执行错误重试 / 友好报错

### Phase 3 — 增强（后续）

- [ ] Streamable HTTP MCP transport
- [ ] Skill ↔ MCP 工具绑定（frontmatter `recommended-tools`）
- [ ] Lens 接入同一 agent runtime
- [ ] Skill / MCP 模板市场（本地索引，无强制商店）

---

## 十、Acceptance Criteria（总体验收）

### 10.1 MCP

- [ ] 用户可添加 stdio MCP Server 并通过「测试连接」看到工具列表
- [ ] 对话中模型可成功调用至少 2 种不同工具（如 fetch + 用户自定义）
- [ ] 禁用 MCP 后，模型请求不再携带 tools 参数
- [ ] 单个 Server 崩溃不会导致 Chat 进程崩溃

### 10.2 Skill

- [ ] 用户可在对话中选择/清除 Skill
- [ ] Skill 正文正确注入 system prompt（可在调试模式验证）
- [ ] 用户目录下新增 `SKILL.md` 后，重启或刷新列表即可见
- [ ] Skill 格式错误时有明确报错，不阻塞应用启动

### 10.3 质量

- [ ] 现有 Chat 功能（纯文本对话、流式、附件图片）无回归
- [ ] Lens / 翻译 / 截图翻译行为无回归
- [ ] 新增 permissions 写入 `src-tauri/capabilities/default.json`（若需 shell 插件 scope）

---

## 十一、Out of Scope（明确不做）

| 项 | 原因 |
|----|------|
| Lens ↔ Chat 截图传递 | 下一里程碑 |
| 完整 Agent 自主规划（多步骤无人值守） | 偏离轻量定位，接近 AionUi |
| 知识库 RAG / 向量库 | 独立大功能 |
| Ollama 本地模型 | 独立 PRD |
| MCP Resources / Prompts 完整实现 | MVP 仅 Tools |
| 内置 MCP 市场联网下载 | Phase 3+ |
| Linux 平台 | 现有产品范围外 |
| 云端同步 Skill/MCP 配置 | 与本地优先冲突 |

---

## 十二、Risks & Mitigations（风险）

| 风险 | 影响 | 缓解 |
|------|------|------|
| Provider tools 格式不一致 | 部分 API 无法调用 | 能力检测 + UI 禁用 MCP |
| MCP 子进程安全 | 恶意 Server 读写文件 | 确认框、文档警示、可选 sandbox 路径白名单（P2） |
| Context 膨胀（工具 schema 过多） | 请求失败 / 费用高 | 按 Server 筛选 tools；Skill 限制工具集 |
| Agent 死循环 | 卡死、费 token | `maxToolRounds` + 超时 + 取消 |
| Rust MCP 生态不成熟 | 延期 | Phase 0 Spike；必要时 MVP 仅 native tools + Skill |

---

## 十三、Resolved Decisions & Remaining Questions（已决策与待后续）

### 13.1 已决策

| # | 问题 | 决策 |
|---|------|------|
| 1 | MCP 配置是否与 Cursor 共用 `~/.cursor/mcp.json`？ | **否**；提供「从该文件导入」即可，不直接共用。 |
| 2 | 内置 web_search 是否算作 MCP？ | **否**；作为 Native Tool，UI 和 agent loop 与 MCP Tool 统一。 |
| 3 | 对话是否默认启用 MCP？ | **否**；设置中手动开启，导入 server 后也默认 disabled。 |
| 4 | Tool 调用如何持久化？ | 存在对应 assistant message 的 `tool_calls` 元数据中。 |
| 5 | Skill 作用域？ | 会话固定、可切换；conversation 保存 `active_skill_id`。 |
| 6 | Tool 确认策略？ | 读类自动，敏感工具确认。 |

### 13.2 待后续阶段决策

| # | 问题 | 建议默认 |
|---|------|----------|
| 1 | Skill 是否允许项目级 `.kivio/skills/`？ | **是**（Phase 2），MVP 仅 app_data |
| 2 | 是否支持用户自定义 Skill 在线编辑？ | Phase 2 只读预览；编辑器 Phase 3 |
| 3 | Chat 是否需要独立 web_search 凭据？ | MVP 复用 Lens web_search settings；后续按使用反馈拆分 |

---

## 十四、Success Metrics（成功指标）

| 指标 | 目标 |
|------|------|
| MCP 配置成功率 | > 80%（测试连接通过） |
| 含 Tool 对话完成率 | > 90%（无 unhandled error） |
| Skill 使用率 | 活跃 Chat 用户 20%+ 曾选择 Skill |
| 回归 | 零 P0 bug（流式、持久化） |

---

## 十五、Related Files（实施参考）

| 文件 | 用途 |
|------|------|
| `src/chat/Chat.tsx` | 接入 SkillSelector、tool 事件 |
| `src/chat/InputBar.tsx` | Skill / 工具状态入口 |
| `src-tauri/src/chat/commands.rs` | agent loop 入口 |
| `src-tauri/src/api.rs` | tools 参数 SSE 解析 |
| `src-tauri/src/web_search.rs` | native tool 参考 |
| `src/lens/WebSearchBlock.tsx` | Tool UI 参考 |
| `src/settings/SettingsShell.tsx` | 新 Tab 或 Chat 子页 |
| `src/api/tauri.ts` | 契约 |
| `.trellis/spec/guides/cross-layer-thinking-guide.md` | 跨层设计 |

---

## 十六、变更记录

| 日期 | 版本 | 变更 |
|------|------|------|
| 2026-06-03 | v1.0 | 初始版本：Chat MCP + Skill PRD |
| 2026-06-03 | v1.1 | 基于多 agent 仓库调研补充实现契约：assistant 内嵌 tool_calls、会话级 Skill、读类自动/敏感确认、Chat 专用 run 事件与 snake_case/camelCase 存储边界 |

---

**文档状态**: ✅ 待评审  
**建议下一步**:

1. 评审 v1.1 已决策与后续问题（第十三节）  
2. 执行 Phase 0 Spike（MCP stdio POC + Skill loader）  
3. 在 `.trellis/tasks/` 创建实现任务并关联本 PRD  

---

*本文档为 Kivio Chat 能力增强专项 PRD，与桌面《Kivio AI 客户端产品需求文档 PRD》互补：后者覆盖全局路线，本文档聚焦 MCP 与 Skill 的实现范围与分期。*
