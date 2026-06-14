# Chat Agent Plan Mode

## Goal

Add a Plan mode to Kivio Chat so the agent can investigate, ask clarifying questions, and produce an implementation plan before taking side-effecting actions. The feature should feel like Codex / Claude Code / OpenCode / Cline style "plan first, act after approval", not like a user task manager.

## Product Boundary

- Plan mode is an agent runtime permission mode.
- It is separate from the existing agent todo list. Todo tracks execution progress; Plan mode controls whether execution is allowed.
- Plan mode must not create reminders, calendar items, or user-editable tasks.
- Users should be able to enter Plan mode and later approve or switch back to normal execution.
- The current conversation should carry enough plan context into the next turn so the agent can continue from the accepted or revised plan.

## External Research Summary

### Codex

Codex documentation describes Plan mode as a way to gather context, ask clarifying questions, and build a stronger plan before implementation. It can be toggled with `/plan` or Shift+Tab. Codex also frames planning as useful for complex or ambiguous work, while normal execution remains appropriate for small, clear edits.

Relevant sources:
- https://developers.openai.com/codex/codex-manual.md
- https://developers.openai.com/codex/codex-manual.md#execution-model-and-workflows
- https://developers.openai.com/codex/codex-manual.md#approvals-sandboxing-and-security

### Claude Code

Claude Code has an explicit `plan` permission mode. Official docs say plan mode runs read-only tools and lets Claude analyze and plan without editing source files. Claude can clarify requirements before finalizing the plan. The user can approve the plan, keep planning with feedback, or exit plan mode.

Relevant sources:
- https://code.claude.com/docs/en/permission-modes
- https://code.claude.com/docs/en/agent-sdk/permissions
- https://docs.anthropic.com/en/docs/claude-code/common-workflows

### OpenCode

OpenCode models Plan as a primary agent with restricted permissions. Build is the default full-access primary agent; Plan is for planning and analysis. OpenCode's permission model can set edit and bash permissions to ask/allow/deny, and examples show a Plan agent denying edit and bash.

Relevant source:
- https://opencode.ai/docs/agents/

### Cline

Cline uses a Plan / Act split. Plan mode can read the codebase, search, and discuss strategy, but cannot modify files or execute commands. Act mode preserves the planning conversation context and then executes the plan. Cline also supports separate models for Plan and Act.

Relevant source:
- https://docs.cline.bot/core-workflows/plan-and-act

### Roo Code

Roo Code implements multiple modes with distinct tool groups. Architect mode is the closest planning mode: it has read and MCP access plus restricted markdown editing. Ask mode is read/MCP only. Code mode has full read/edit/command/MCP access.

Relevant source:
- https://docs.roocode.com/basic-usage/using-modes

### OpenAI Agents SDK

The Agents SDK frames agents as model + instructions + tools + guardrails + handoffs. It recommends starting with the smallest single agent and splitting only when instructions, tool surfaces, approval policies, or ownership differ materially. For Kivio Plan mode, this argues against a heavy multi-agent rewrite at MVP.

Relevant sources:
- https://developers.openai.com/api/docs/guides/agents/define-agents
- https://developers.openai.com/api/docs/guides/agents/orchestration
- https://openai.github.io/openai-agents-python/handoffs/

### LangGraph

LangGraph is useful as a reference for interrupt/resume and stateful workflows. Its human-in-the-loop pattern stores state through checkpoints, pauses for user input, and resumes with a command. Kivio likely does not need a graph runtime for MVP, but the pause/resume idea is directly relevant to plan approval.

Relevant source:
- https://docs.langchain.com/oss/python/langgraph/thinking-in-langgraph

## Current Kivio Runtime Observations

- Chat already has a Rust-native agent loop in `src-tauri/src/chat/agent/loop_.rs`.
- Tool availability is assembled in `src-tauri/src/chat/commands.rs`, then filtered by assistant presets, data connector filters, skill filters, and inline-code request filters.
- User-facing tool approval policy already exists in settings as `chatTools.approvalPolicy` and is surfaced in `src/chat/InputBar.tsx`.
- Agent-owned persisted state now exists via `Conversation.agent_todo_state`; this gives a proven pattern for adding conversation-scoped runtime state.
- Context estimation already supports runtime prompt segments, including the recently added `agent_todo` segment.
- Tool execution approval and serial/parallel scheduling are centralized enough that Plan mode can be enforced before execution rather than only by prompt text.

## Recommended MVP

Implement Plan mode as a conversation/runtime mode with hard tool gating.

1. Add conversation-level plan state.
   - `Conversation.agent_plan_state`
   - fields likely needed: `mode`, `plan`, `status`, `updated_at`, maybe `approved_at`
   - mode values: `act`, `plan`
   - status values: `draft`, `approved`, `stale` or keep minimal for MVP

2. Add a UI mode toggle near the composer.
   - Default: Act
   - Plan mode visually distinct but compact.
   - No user editing of the generated plan in MVP unless later requested.

3. Inject a Plan mode runtime prompt segment.
   - Explain that Plan mode should investigate and produce a concise plan.
   - It may ask clarifying questions.
   - It must not claim to have changed files or executed side-effecting actions.

4. Enforce tool restrictions in backend.
   - Allow read-only discovery tools where available.
   - Deny or omit side-effecting tools: write/edit files, run_command, run_python if treated as execution, memory mutation, image generation, unknown/non-read-only MCP tools, and other write-like native tools.
   - Keep `todo_write` / `todo_update` available or decide explicitly; recommendation: keep them available because they update agent progress, but make clear they are not plan approval.

5. Add plan capture.
   - Easiest MVP: assistant final text is the plan; persist it as `agent_plan_state.plan` after a Plan mode turn.
   - Stronger option: add native `plan_write` tool for structured plan state. This mirrors todo tooling but may be more work.

6. Add approval / continue behavior.
   - User can switch to Act and say to execute the plan.
   - Optionally add a button like "Execute plan" that switches mode to Act and sends a short continuation prompt.
   - The accepted plan must be injected into the next Act turn as context.

## Open Design Decision

Plan mode needs one product decision before implementation:

Should Kivio MVP make plan approval explicit with a button/state (`Approve plan` / `Execute plan`), or simply let the user switch from Plan to Act and say "start"?

Recommendation: add an explicit `Execute plan` affordance if the current plan exists. It matches Claude Code/Cline expectations and reduces ambiguity, while still allowing manual mode switching.

## Acceptance Criteria

- User can enter Plan mode from Chat UI.
- In Plan mode, the agent can analyze and produce a plan without modifying workspace/user files or running side-effecting tools.
- Plan context persists on the conversation and is visible to the next turn.
- User can leave Plan mode and ask the agent to execute the plan with the plan injected as context.
- Tool records should show blocked/skipped behavior clearly if the model attempts a disallowed action in Plan mode.
- Existing Act behavior remains unchanged when Plan mode is off.

## Out of Scope for MVP

- User-editable plan document.
- Calendar/reminder/task-management behavior.
- Full multi-agent orchestration.
- Separate Plan/Act model configuration.
- Browser/cloud "ultraplan" style offloading.
- Complex graph runtime adoption.

## Technical Notes

- Prefer reusing the existing chat runtime, prompt segment, tool filtering, event, and conversation persistence patterns.
- Do not rely on prompt-only safety; enforce Plan mode at tool availability/execution level.
- If plan state is updated during a tool loop, follow the same merge-before-save rule used for `agent_todo_state`.
- Any new conversation field must use `#[serde(default)]` for old conversation compatibility.
- If a Tauri event is added, mirror frontend type definitions in `src/api/tauri.ts` and `src/chat/types.ts`.
