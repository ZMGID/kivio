# External Agent Integration

## Scenario: Project-scoped discovery and safe process shutdown

### 1. Scope / Trigger

Apply this contract when changing external CLI detection, model selection, project cwd propagation,
agent caches, or a protocol's end-of-turn lifecycle.

The result may depend on both user-global CLI config and project-local files. A protocol completion
event may also precede the child process's final stdout flush.

### 2. Signatures

Frontend API:

```ts
chatApi.detectExternalAgents(
  forceRefresh?: boolean,
  conversationId?: string | null,
): Promise<DetectedExternalAgent[]>
```

Tauri and detection boundary:

```rust
async fn chat_detect_external_agents(
    app: AppHandle,
    state: State<'_, AppState>,
    force_refresh: Option<bool>,
    conversation_id: Option<String>,
) -> Result<Value, String>;

async fn detect_all_agents(cwd: &Path) -> Vec<DetectedAgent>;
async fn detect_single_agent(def: &RuntimeAgentDef, cwd: &Path) -> DetectedAgent;
```

Pi RPC boundary:

```rust
async fn run_pi_rpc_session(
    child: &mut Child,
    prompt: &str,
    model: Option<&str>,
    sink: impl FnMut(UnifiedAgentEvent),
    cancel_check: impl Fn() -> bool,
) -> Result<(), String>;
```

### 3. Contracts

- `conversationId` present: load the conversation, resolve its effective project/workspace cwd, and
  use that cwd for every model probe.
- `conversationId` absent: use the process current directory. The CLI still owns global-config
  discovery.
- OpenCode probe order: native `opencode models` -> ACP `session/new` -> static definition fallback.
- Kivio must not parse OpenCode JSONC or reproduce provider/config merge rules.
- Full detection cache key: `cwd`.
- Model metadata cache key: `agent_id:cwd`.
- A frontend context change clears prior results; late responses from the old context are ignored.
- Pi `agent_end` is logical completion only. Close stdin to trigger Pi shutdown, then keep draining
  stdout until EOF so queued writes can flush.

### 4. Validation & Error Matrix

| Condition | Required behavior |
|---|---|
| Conversation ID cannot be loaded | Return command error; do not silently use another project's cwd |
| `opencode models` times out, exits nonzero, or has no valid IDs | Try ACP discovery |
| ACP returns no usable models | Use the agent's static fallback list |
| Native output contains empty, duplicate, whitespace, or non-`provider/model` lines | Ignore invalid lines and deduplicate full IDs |
| Pi stdout read fails before `agent_end` | Return an error |
| User cancels before or after `agent_end` | Kill the child and return `cancelled` |
| Pi writes protocol lines after `agent_end` | Keep the pipe open, drain the lines, and do not surface them as a new turn |

### 5. Good / Base / Bad Cases

- **Good**: Project A defines `custom/model-a`; the selector shows it only while the conversation
  resolves to Project A.
- **Base**: No conversation is open; detection uses current cwd and still lists global OpenCode
  models.
- **Bad**: Detection runs in a temp directory, caches globally, and project B temporarily shows
  Project A's models.
- **Good**: Pi emits `agent_end`, flushes a final response, closes stdout, and Kivio returns success.
- **Bad**: Kivio drops stdout immediately at `agent_end`, causing Pi's final write to fail with
  `EPIPE`.

### 6. Tests Required

- Parser test: accept custom providers and multi-segment model IDs.
- Parser test: reject invalid/empty output and deduplicate IDs.
- Cache test: a value stored under cwd A is not returned for cwd B.
- UI behavior: a conversation change clears previous results and ignores stale promises.
- Pi regression: write `agent_end`, delay, write a trailing line, and assert the writer does not see
  a broken pipe.
- Pi cancellation: keep stdout open after `agent_end`, activate cancellation, and assert
  `cancelled`.
- Run full Rust tests, frontend tests, TypeScript type-check, and ESLint.

### 7. Wrong vs Correct

#### Wrong

```rust
if event == AgentEnd {
    break; // Drops stdout before the child flushes.
}

let cwd = std::env::temp_dir(); // Drops project-local CLI config.
let cache_key = agent_id;       // Cross-project cache collision.
```

#### Correct

```rust
if event == AgentEnd {
    let _ = stdin.shutdown().await;
    agent_ended = true;
    // Continue reading until stdout EOF; cancellation remains active.
}

let cwd = resolve_effective_cwd(app, conversation_id, project_id)?;
let cache_key = format!("{agent_id}:{}", cwd.to_string_lossy());
```
