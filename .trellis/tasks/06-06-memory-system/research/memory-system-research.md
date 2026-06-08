# Kivio Memory System Research

## Scope

This research compares Kivio's current chat architecture with Hermes Agent's memory implementation and broader agent memory patterns. The goal is to design a memory system that fits Kivio's lightweight desktop architecture without forcing a premature heavyweight vector stack.

## Kivio Findings

- Kivio Chat currently stores one JSON file per conversation plus `index.json` under the app data `conversations/` directory. This is implemented in `src-tauri/src/chat/storage.rs`.
- Conversation context already has a short-term compression layer: `ConversationContextSummary` and `ConversationContextState` in `src-tauri/src/chat/types.rs`.
- Older messages are compressed by `compress_conversation_context()` in `src-tauri/src/chat/commands.rs`. Its prompt already asks the model to preserve user goals, preferences, constraints, decisions, file paths, commands, tool results, unresolved questions, and important facts.
- Prompt assembly happens in `src-tauri/src/chat/agent/prepare.rs` via `build_chat_system_prompt_with_segments()`. This is the cleanest place to inject a compact memory block and account for its token segment.
- Tool registration flows through `src-tauri/src/mcp/registry.rs` and `ChatToolDefinition`, with native tools, skills, and MCP tools sharing one tool surface. A memory tool can fit here without changing model-provider APIs.
- There is no existing SQLite, vector DB, or embedding dependency in `src-tauri/Cargo.toml` or `package.json`.

## Local Hermes Findings

Local deployment:

- Code: `/Users/zmair/.hermes/hermes-agent`
- Runtime state: `/Users/zmair/.local/state/hermes`
- Main DB: `/Users/zmair/.hermes/state.db`
- LaunchAgents: `/Users/zmair/Library/LaunchAgents/ai.hermes.gateway.plist` and `ai.hermes.dashboard.plist`
- Local notes: `/Users/zmair/.claude/projects/-Users-zmair/memory/hermes_agent_setup.md` and Obsidian Hermes notes.

Relevant Hermes implementation patterns:

- `tools/memory_tool.py` implements bounded curated memory in two profile-scoped files:
  - `MEMORY.md`: agent personal notes, environment facts, project conventions, lessons.
  - `USER.md`: user profile, communication preferences, workflow habits.
- Hermes freezes the memory snapshot at session start for prompt-cache stability. Mid-session writes are durable on disk but do not mutate the already-cached prompt.
- Hermes caps memory by character count, rejects exact duplicates, uses substring-based replace/remove, detects external file drift, and scans memory entries for prompt injection/exfiltration before accepting them.
- `hermes_state.py` implements `state.db` with SQLite, WAL mode, `sessions`, `messages`, FTS5, and trigram FTS for CJK/substring search.
- Hermes treats FTS session search as separate from curated memory: memory is small and always injected; session search is unlimited and on-demand.
- `agent/memory_manager.py` defines a provider lifecycle:
  - `system_prompt_block()`
  - `prefetch(query)`
  - `queue_prefetch(query)`
  - `sync_turn(user, assistant, messages)`
  - `on_session_end(messages)`
  - `on_pre_compress(messages)`
  - `on_memory_write(...)`
- Hermes allows only one external memory provider at a time to avoid tool-schema bloat and conflicting memory backends.
- `agent/conversation_loop.py` prefetches external memory once per user turn, fences it inside `<memory-context>...</memory-context>`, injects it into the current user message only for the API call, then syncs the completed turn after response.

## External Research

Sources used:

- Hermes Persistent Memory docs: https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/features/memory.md
- Hermes Memory Providers docs: https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/features/memory-providers.md
- Hermes Session Storage docs: https://github.com/NousResearch/hermes-agent/blob/main/website/docs/developer-guide/session-storage.md
- Letta Code Memory docs: https://docs.letta.com/letta-code/memory/
- Mem0 project/docs discovery: https://docs.mem0.ai and https://github.com/mem0ai/mem0

Common pattern across Hermes, Letta, and Mem0:

- Split memory into durable identity/profile facts, session/conversation recall, and procedural/project knowledge.
- Keep always-injected memory small and high-signal.
- Use on-demand retrieval for long transcripts or semantic recall.
- Let the agent write memories, but provide user-visible management, deletion, and safety controls.
- Run reflection/consolidation outside the main response path, commonly after N turns, on compaction, or at session end.

## Design Implications For Kivio

- Kivio should not start with a heavy vector DB. Its current storage model is simple, portable, and aligned with its small package/low footprint goal.
- The first memory layer should be file-backed, bounded, editable, and injected into chat prompts.
- A second layer should add deterministic conversation recall. The simplest MVP can scan current conversation JSON files; a later version can add SQLite/FTS for speed and CJK search.
- Semantic memory/provider support should be a plugin/provider interface, not an MVP requirement.
- Memory must be part of context accounting so users can understand token cost and compression behavior.

