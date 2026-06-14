# Agent Plan Mode Research Notes

## Pattern Across Tools

The common product shape is:

- Plan mode is a mode/permission boundary, not a todo list.
- The model can inspect, search, ask questions, and draft a plan.
- Writes, file edits, shell execution, and other side effects are blocked, denied, or require approval.
- The plan is reviewable by the user.
- Execution happens only after mode switch or plan approval.
- The planning context carries into execution.

## Source Highlights

- Codex: Plan mode gathers context, asks clarifying questions, and builds a stronger plan before implementation. Toggle with `/plan` or Shift+Tab.
- Claude Code: `plan` permission mode runs read-only tools; Claude analyzes and plans without editing source files. Approval exits plan mode into an execution permission mode.
- OpenCode: Plan is a restricted primary agent; Build is full-access. Permissions can allow/ask/deny edit and bash.
- Cline: Plan mode can read/search/discuss but cannot modify files or execute commands; Act mode retains context and executes.
- Roo Code: Modes are tool-surface configurations. Architect is a planning-oriented mode with read/MCP and restricted markdown edit.
- OpenAI Agents SDK: split agents only when tool surfaces, instructions, approval policies, model choice, or reply ownership materially differ.
- LangGraph: useful reference for human review, interrupt, state checkpointing, and resume, but likely too heavy for the MVP.

## Kivio Implementation Implication

The best MVP is not a separate agent framework. It is a mode flag plus hard tool gating inside the existing Chat runtime.

Use existing patterns:

- Conversation field with serde default.
- Runtime prompt segment included in request and context estimate.
- Backend-side tool filtering before model call.
- Backend-side execution guard for safety if a disallowed tool is still requested.
- Frontend state updates through Tauri event if plan state changes during a run.

## Suggested Plan State Shape

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AgentPlanState {
    #[serde(default)]
    pub mode: AgentPlanMode,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub status: AgentPlanStatus,
    #[serde(default)]
    pub updated_at: i64,
}
```

Possible enums:

```rust
#[serde(rename_all = "snake_case")]
pub enum AgentPlanMode {
    Act,
    Plan,
}

#[serde(rename_all = "snake_case")]
pub enum AgentPlanStatus {
    Empty,
    Draft,
    Approved,
}
```

MVP can simplify further:

- Store `mode: String`
- Store `plan: Option<String>`
- Store `updated_at`

## Tool Policy Recommendation

Allow in Plan mode:

- Read/search/fetch type tools with explicit read-only semantics.
- Skill activation only if it does not execute side effects.
- Todo tools, if treated as internal state updates.

Block in Plan mode:

- Native file writes/edits.
- Command execution.
- Python/code execution if it can perform computation with side effects or confuse the "no execution" promise.
- Memory mutation.
- Image generation.
- Non-read-only MCP tools.
- Unknown tools.

## Main Risk

If Plan mode only changes the system prompt, a model may still call an available write/command tool. The backend must enforce the mode at the tool boundary.

## Useful URLs

- Codex manual: https://developers.openai.com/codex/codex-manual.md
- Claude Code permission modes: https://code.claude.com/docs/en/permission-modes
- Claude Agent SDK permissions: https://code.claude.com/docs/en/agent-sdk/permissions
- OpenCode agents: https://opencode.ai/docs/agents/
- Cline Plan and Act: https://docs.cline.bot/core-workflows/plan-and-act
- Roo Code modes: https://docs.roocode.com/basic-usage/using-modes
- OpenAI Agents SDK agent definitions: https://developers.openai.com/api/docs/guides/agents/define-agents
- OpenAI Agents SDK orchestration: https://developers.openai.com/api/docs/guides/agents/orchestration
- LangGraph thinking guide: https://docs.langchain.com/oss/python/langgraph/thinking-in-langgraph
