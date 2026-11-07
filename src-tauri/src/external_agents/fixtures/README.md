# External CLI offline replay corpus

These fixtures drive the existing native session code without an installed CLI, login, network request, or model charge. The test-only peer replaces native I/O. It does not implement another session runtime.

Run the record-backed and synthetic replay tests with `cargo test --manifest-path src-tauri/Cargo.toml offline_replay`. Run the bounded loader check with `cargo test --manifest-path src-tauri/Cargo.toml replay_loader`. The ordinary Rust suite includes both on macOS and Windows.

On Windows, use the existing launcher to supply the Common-Controls v6 manifest required by local test executables.

```powershell
./scripts/win-cargo-test.ps1 --lib offline_replay
./scripts/win-cargo-test.ps1 --lib replay_loader
```

`expect_outbound` matches the listed object fields recursively. Unlisted fields are allowed, while arrays, values, and RPC ID types must match exactly. `emit_inbound` writes its frame unchanged. Headers declare the protocol, version, and capture classification. The loader accepts at most 512 KiB per transcript, 64 KiB per frame, and 1024 steps. Test waits have a five-second deadline. Pipe peers run the current Rust test executable and exit before libtest can append non-protocol text.

## Sources and transformations

T3 records come from [pingdotgg/t3code at 85bd3f2a2a0ad4a8ddf2950e01db80765e9a1bf4](https://github.com/pingdotgg/t3code/tree/85bd3f2a2a0ad4a8ddf2950e01db80765e9a1bf4/apps/server/src/orchestration-v2/testkit/fixtures/simple). The copied [MIT license](T3-LICENSE) applies to those excerpts. `source_line` is the original one-based NDJSON line. Existing upstream sanitization is retained. Public fixture prompts and probe text are intentional test data.

| Fixture | Classification and version | Exact source and changes | Production check |
| --- | --- | --- | --- |
| `codex-real-turn.ndjson` | Real native turn excerpt. Codex CLI 0.156.1. | `simple/codex_transcript.ndjson`, lines 15, 17, 19, 20, 22 through 27, and 29. Inbound frames are unchanged. The actor starts with the captured native thread and request ID 3. A Kivio outbound subset replaces T3's client request expectation and uses the inert `/replay` working directory. Initialization and thread creation are outside this excerpt. | `CodexAppServerSession::run_turn` checks outbound prompt, approval policy, thread identity, text, native usage, and delivery. |
| `pi-real-turn.ndjson` | Real native turn excerpt. Pi 1.0.0. | `simple/pi_transcript.ndjson`, lines 16 through 38. Inbound frames are unchanged. The source's `streamingBehavior` expectation becomes Kivio's own prompt expectation with ID 1. T3 extension startup and post-turn queries are omitted. | `PiRpcSession` and its production actor emit the recorded text and usage through native `agent_settled`. Native diagnostics retain the same connection and generation across the actor's nested task while omitting payload text. |
| `acp-adapted-grok-turn.ndjson` | Adapted public capture. Grok CLI 1.0.41, ACP protocol 1. | `simple/grok_transcript.ndjson`, lines 30 through 50, 53 through 65, and 72. T3's `kind` envelope becomes JSON-RPC 2.0. The final response receives request ID 7 and loses its synthetic `method`. Native payload bodies remain unchanged. Only thought chunks, message chunks, and the final result are selected. Setup, commands, account metadata, and unrelated extension notifications are omitted. | `acp_read_prompt` checks text, reasoning, and a valid terminal response for the native session. This is reader coverage, not an ACP launch or prompt-write check. |
| `claude-recorded-approval.ndjson` | Real approval excerpt with synthetic surrounding turn. Claude CLI 2.1.220. | Existing `claude_stream.rs::a_can_use_tool_request_becomes_a_pending_ask_when_a_host_can_answer`, captured on 2026-07-29 using `--permission-prompt-tool stdio --permission-mode default`. The recorded `can_use_tool` frame is unchanged. User input, initialization, answer, and result frames are synthetic. | `ClaudeStreamJsonSession::run_turn` rejects the recorded request when no host can answer, completes the turn, and emits its text. The peer checks the correlated fail-closed response. |
| `claude-disconnect.ndjson` | Synthetic. Claude stream JSON. | Peer exits only after reading the user prompt. | The production session reports an error while retaining `possibly_sent`, so callers cannot treat EOF as proof the prompt was unsent. |
| `dsh-two-turns.ndjson` | Synthetic. Existing dsh rc.8 JSON-RPC shape. | Hand-authored prompt ACK, status, text, terminal, and idle notifications. | One `DshJsonRpcSession` serves two ordinary prompts. The peer checks native session and prompt contents. |
| `dsh-stop-next.ndjson` | Synthetic. Existing dsh rc.8 JSON-RPC shape. | Peer waits for the actual cancel request before acknowledging it, then accepts the next prompt. | Cancellation is sent after the first output. The session reports `cancelled`, retains delivery, and serves the next prompt on the same connection. |
| `dsh-disconnect.ndjson` | Synthetic. Existing dsh rc.8 JSON-RPC shape. | Peer exits after receiving an ordinary prompt. | EOF reports an error and preserves `possibly_sent`. |
| `antigravity-steps.ndjson` | Synthetic. Existing Antigravity 1.1.26 step shape. | Hand-authored tool lifecycle, streamed text, result, and usage. | The production `TurnStream` parser emits exactly one tool result and answer and reconciles usage. This is parser coverage, not an offline process or actor replay. |

T3's Claude record uses `claude-agent-sdk.query`. It is not Claude CLI stream JSON and is deliberately excluded. T3's Cursor SDK record is likewise excluded from ACP wire evidence. No private CLI history or credentials were read to build this corpus.

## Scenario coverage and limits

The nine scenarios below combine the new corpus with existing ordinary tests. A fixture for one protocol does not prove the same capability for every provider. Names refer to tests in `external_agents` unless another module is shown. Live tests marked `#[ignore]` are excluded from offline evidence.

| Scenario | Offline evidence | Remaining capture or capability limit |
| --- | --- | --- |
| Multiple turns | New `offline_replay_dsh_two_prompts_and_disconnect_keep_native_delivery`. Existing Pi `persistent_io_serves_two_turns_on_the_same_streams`, Codex `runtime_correlates_exact_replies_and_native_turns_and_marks_uncertain_delivery`, and Claude `priority_steering_waits_for_echo_and_survives_superseded_result`. | Recorded Codex, Pi, and adapted ACP excerpts cover one turn each. Antigravity has parser coverage only. |
| Resume | ACP import replay tests use the production `session/load` reader. `external_history_publication_and_recovery_on_real_files` covers retained native binding, resume publication, and recovery. | This corpus does not contain an entire reconnect recording. dsh `classifies_only_missing_resume_targets_for_fresh_fallback` checks classification only. |
| Stop | New dsh stop acknowledgement and next prompt. Pi `persistent_abort_drains_to_settled_and_returns_cancelled`, `abort_sends_clear_queue_before_abort`, and `pi_side_abort_returns_cancelled_and_keeps_next_turn_usable`. ACP `live_cancel_sends_a_notification` is an offline duplex test. Claude `cancelling_rejects_every_pending_approval_exactly_once` checks pending decisions. | Antigravity cancellation loses the session by design. Native stop captures are still missing from this corpus. |
| Steering | Claude `priority_steering_waits_for_echo_and_survives_superseded_result` drives the production actor through receipt echo, superseded result, and another turn. Pi `handled_injection_is_acknowledged_without_being_resent` covers acknowledgements. dsh tests cover acknowledged-turn gating. | ACP and Antigravity do not expose native steering. No synthetic native steering capability is invented for them. |
| Approval | New recorded Claude fail-closed excerpt. ACP `live_prompt_ignores_other_sessions_and_only_finishes_on_its_response` covers permission correlation and stale decisions. Pi `extension_ui_confirm_bridges_user_decision_and_fails_closed_without_host` covers the extension bridge. Codex and dsh retain existing reverse-request and response-shape tests. | Only the Claude request excerpt has a directly recorded approval payload here. The surrounding turn is synthetic. |
| Disconnect | New Claude and dsh real-pipe EOF cases preserve uncertain delivery. Codex `runtime_correlates_exact_replies_and_native_turns_and_marks_uncertain_delivery` has its existing drop mode. Pi `persistent_eof_before_settled_is_an_error` covers missing settlement. `run.rs::persistent_dispatch_only_retries_when_actor_reports_unsent_failure` checks the common retry decision. | No real failure capture is claimed. ACP and Antigravity fault coverage remains in their existing reader and parser tests. |
| Fork | `external_history_publication_and_recovery_on_real_files` checks a fresh native context, one visible-history handoff, and source-session preservation. Pi `regenerate_forks_pi_before_resubmitting_the_visible_user_message` exercises native fork ordering. | Common text handoff is distinct from native fork. The corpus contains no native fork capture. |
| Rewind | The common real-file lifecycle test checks the pending rewind transaction and late delivery isolation. Pi `send_after_rewind_forks_before_the_first_removed_turn` and Codex alignment tests cover native boundaries. | Not every provider supports native rewind. ADR-0006 defines the fresh-session handoff fallback. No native rewind capture is included. |
| Late background events | Claude's priority-steering actor test includes an idle task wake and a later prompt. Pi `stale_settlement_after_compact_cannot_finish_the_next_prompt` and `leftover_btw_entries_cannot_finish_the_next_prompt` cover stale settlement. `background_tasks.rs` checks native task ownership by conversation and process. Codex's identity peer filters stale and child-thread terminal frames. | ACP foreign-session filtering does not imply native background-task support. Antigravity has no background lifecycle capture. |

## Update a native version

1. Capture a benign scenario in an isolated working directory or select a public native recording. Record the CLI version, native protocol, recorder, source URL and immutable revision, original line numbers, and capture classification. Keep credentials and personal history out of the fixture.
2. Curate only the frames needed for the scenario. Preserve recorded inbound payloads when possible. Document every envelope, request ID, path, or prompt transformation. Mark hand-authored fault injection and surrounding frames as synthetic. Retain the source license.
3. Replay through the existing production owner and assert outbound requests, emitted outcomes, delivery state, and terminal behavior. Run `offline_replay` and `replay_loader` using the commands above.
4. Run the relevant ignored live CLI suite only when explicitly enabled for a signed-in CLI and model requests. Record that CLI's exact version and result separately. An old fixture passing does not prove compatibility with an upgraded binary.
5. Run the complete Rust suite with `cargo test --manifest-path src-tauri/Cargo.toml` or `./scripts/win-cargo-test.ps1` on Windows. Update the scenario table with the evidence actually exercised.

Redacted diagnostic summaries omit payload values and are not executable replay transcripts.
