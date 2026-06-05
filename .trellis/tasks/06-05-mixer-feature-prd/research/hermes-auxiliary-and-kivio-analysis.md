# Hermes Auxiliary and Kivio Mixer Research

Date: 2026-06-05

## Scope

Research requested by the user:

- Reference Hermes Agent's Auxiliary behavior.
- Inspect the local Hermes Agent installation.
- Try invoking Hermes Auxiliary once.
- Analyze the current Kivio project and write a PRD for a new "Mixer" feature.

## Local Hermes Findings

The visible candidate directories `/Users/zmair/Documents/Codex/2026-05-25/hermes-agent` and `/Users/zmair/ZM database/hermes-as` were empty. The active local Hermes install is:

- `/Users/zmair/.hermes/hermes-agent`

Running processes showed:

- `python3 -m hermes_cli.main dashboard --tui --no-open`
- `python -m hermes_cli.main gateway run --replace`
- TUI entry: `/Users/zmair/.hermes/hermes-agent/ui-tui/dist/entry.js`

## Hermes Auxiliary Model System

Hermes has a generalized Auxiliary model router rather than a single user-facing chat feature.

Key files:

- `/Users/zmair/.hermes/hermes-agent/agent/auxiliary_client.py`
- `/Users/zmair/.hermes/hermes-agent/hermes_cli/config.py`
- `/Users/zmair/.hermes/hermes-agent/hermes_cli/web_server.py`
- `/Users/zmair/.hermes/hermes-agent/web/src/pages/ModelsPage.tsx`

Config shape observed in `~/.hermes/config.yaml`:

```yaml
auxiliary:
  vision:
    provider: auto
    model: ''
    base_url: ''
    api_key: ''
    timeout: 120
    extra_body: {}
  compression:
    provider: auto
    model: ''
    base_url: ''
    api_key: ''
    timeout: 120
    extra_body: {}
  title_generation:
    provider: auto
    model: ''
    base_url: ''
    api_key: ''
    timeout: 30
    extra_body:
      stream: false
```

Important behavior from `agent/auxiliary_client.py`:

- Per-task overrides are read from `auxiliary.<task>.provider/model/base_url/api_key`.
- Explicit call args win over config.
- `auto` resolves through a provider fallback chain.
- Some failures, including payment/credit exhaustion, can fall back to another provider.
- Each auxiliary task can have its own timeout and `extra_body`.
- The public `call_llm(...)` helper centralizes request formatting and model/provider resolution.

Hermes Dashboard Web UI exposes Auxiliary through the Models page:

- `AuxiliaryTasksModal` lists task slots.
- Each row shows current provider/model or `auto (use main model)`.
- Users can change one task, apply to all tasks, or reset all to `auto`.

REST API:

- `GET /api/model/auxiliary`
- `POST /api/model/set` with `scope: "auxiliary"`

Built-in auxiliary task slots in Hermes Web server:

- `vision`
- `web_extract`
- `compression`
- `skills_hub`
- `approval`
- `mcp`
- `title_generation`
- `triage_specifier`
- `kanban_decomposer`
- `profile_describer`
- `curator`

## Hermes Mixture-of-Agents Tool

Key file:

- `/Users/zmair/.hermes/hermes-agent/tools/mixture_of_agents_tool.py`

Hermes also has a separate MoA tool registered as `mixture_of_agents`.

Observed implementation:

- Reference models are queried in parallel with `asyncio.gather`.
- Successful reference responses are collected.
- Failed models are recorded and tolerated if at least `MIN_SUCCESSFUL_REFERENCES` succeed.
- An aggregator model receives enumerated reference responses in the system prompt and the original user prompt as the user message.
- The returned JSON includes:
  - `success`
  - `response`
  - `models_used.reference_models`
  - `models_used.aggregator_model`
  - `error` on failure

Default constants observed:

- `REFERENCE_MODELS`: Claude Opus, Gemini Pro, GPT Pro, DeepSeek.
- `AGGREGATOR_MODEL`: Claude Opus.
- Reference temperature: `0.6`.
- Aggregator temperature: `0.4`.
- Minimum successful references: `1`.

This is a strong reference for Kivio's "Mixer" if the product goal is multi-model answer synthesis inside Chat.

## Hermes Invocation Attempt

I attempted a minimal `call_llm(task="title_generation")` invocation through the local Hermes install to avoid the 4+1 MoA cost.

Result:

- The call reached Hermes' auxiliary routing path.
- It failed with HTTP 401 `Invalid API Key` from the configured provider.

Takeaway for Kivio:

- Mixer must isolate per-lane failures.
- A single failed provider/key should not necessarily fail the whole run.
- UI should surface which lane failed and continue if the configured minimum successful lanes are met.

## Image / Screenshot Check

The user asked to "look at the image." No separate image was attached in this conversation. I inspected local Hermes image caches:

- `/Users/zmair/.hermes/images/clip_20260526_232324_1.png`
- `/Users/zmair/.hermes/cache/screenshots/*`

The inspected image was not an Auxiliary settings screenshot. Therefore this PRD relies on source/config evidence for Hermes Auxiliary and does not claim screenshot confirmation.

## Current Kivio Architecture Findings

Kivio already has the pieces needed to implement a Chat Mixer without starting from scratch.

Frontend:

- `src/chat/Chat.tsx`
  - Owns send/regenerate flow.
  - Listens to `chat-stream`, `chat-tool`, `chat-tool-confirm`, and `chat-context`.
  - Maintains streaming preview state per conversation.
- `src/chat/InputBar.tsx`
  - Has an existing tool/settings panel affordance using `SlidersHorizontal`.
  - Good candidate for Mixer entry/toggle.
- `src/chat/MessageList.tsx` and `src/chat/MessageBubble.tsx`
  - Render assistant messages, reasoning, and tool cards.
  - Need new rendering for mixer lane responses and final synthesis.
- `src/chat/types.ts`
  - `ChatMessage` already supports metadata fields (`reasoning`, `tool_calls`, `api_messages`, `model_messages`).
  - Needs a `mixer` or `mixer_runs` metadata field.
- `src/api/tauri.ts`
  - Centralizes event listener and Tauri command types.
  - Needs mixer event payload types.

Backend:

- `src-tauri/src/settings.rs`
  - `ModelProvider` supports `supports_tools`, `api_format`, failover keys, enabled models, and model overrides.
  - `DefaultModelsConfig` already has `chat`, `title_summary`, and `compression`.
  - `ChatToolsConfig` handles tool-related settings.
  - Needs `ChatMixerConfig`, likely under `settings.chat` or as a sibling `settings.chat_mixer`.
- `src-tauri/src/chat/commands.rs`
  - `chat_send_message` persists the user message, computes context, then calls `complete_assistant_reply`.
  - `complete_assistant_reply` resolves provider/model, skill, tools, system prompt, runtime messages, and calls `run_agent_loop`.
  - Good place to route to normal chat vs mixer mode.
- `src-tauri/src/chat/agent/`
  - Current runtime is structured around a single provider/model run.
  - `AgentRunConfig` includes provider/model/runtime messages/tools/settings.
  - New mixer orchestration can either call `run_agent_loop` for each lane or introduce a no-tool/non-stream helper for lane calls.
- `src-tauri/src/chat/model/`
  - Provider abstraction already supports OpenAI-compatible, Anthropic Messages, and Apple local provider.
  - Mixer should reuse this abstraction instead of calling `api.rs` directly.

Related existing docs/tasks:

- `docs/CHAT_AGENT_RUNTIME_PRD.md`
- `.trellis/tasks/06-05-chat-agent-runtime-prd/prd.md`
- `.trellis/tasks/06-03-kivio-chat-mcp-skill-implementation/prd.md`
- `.trellis/tasks/06-04-assistant-center-feature/prd.md`

## Product Interpretation

Recommended interpretation for Kivio:

Mixer is a Chat feature that runs multiple selected models in parallel, shows each lane's draft/answer, then optionally asks a selected aggregator model to synthesize a final answer.

This maps to Hermes' MoA tool for reasoning behavior and Hermes Auxiliary for configuration patterns:

- From Hermes Auxiliary: per-task/provider assignment, `auto`, timeout, reset, failure visibility.
- From Hermes MoA: parallel reference calls, minimum successful lanes, aggregator synthesis, model provenance.

## Implementation Constraints

- MVP should avoid tools in mixer lanes. Tool calls multiply risk, approvals, and state complexity.
- MVP should disable or degrade mixer when attachments include images and a selected lane model is not vision capable.
- Apple local provider should not be a default mixer lane because current chat logic short-circuits Apple and does not support images/tools.
- Mixer metadata must be persisted with the assistant message so reload/history restore the lane outputs.
- Streaming event payloads must include `conversation_id`, `run_id`, `message_id`, and lane identifiers to avoid collisions with existing per-conversation streaming state.
