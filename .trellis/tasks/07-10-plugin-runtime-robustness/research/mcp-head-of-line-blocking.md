# MCP stdio head-of-line blocking investigation (2026-07-11)

## Incident

A real OfficeCLI 1.0.135 MCP session produced this sequence:

1. A batch continued after one or more invalid entries and partially committed valid edits.
2. The next `tools/call` mutation executed, but its JSON-RPC response was not delivered.
3. OfficeCLI remained alive and could process later requests.
4. Kivio held `Mutex<McpSession>` while awaiting the missing response, so all later calls to the same server waited for the full 120-second tool timeout.

The long stall was therefore a cross-layer failure: an upstream lost response was amplified by Kivio's session-wide response lock. Kivio live preview was not required to reproduce it.

## Evidence

Conversation artifact:

`%APPDATA%/com.zmair.kivio/conversations/conv_c82645ff-9092-4c9e-a603-de22507d418d.json`

Observed production timings included a partial batch completing in about 180 ms, one following call timing out after about 120001 ms, and a later call succeeding only after the first timeout released the session lock.

Direct MCP probing with `officecli.exe mcp` showed that the server could lose one response and still read and answer a later JSON-RPC request. This separated the OfficeCLI response-loss bug from Kivio's head-of-line amplification.

## Root cause

The old stdio path guarded the complete `tools/call` lifecycle with `Arc<Mutex<McpSession>>`. Although the client already routed responses by request id, request B could not reach the connection while request A held the session mutex awaiting its response.

Root-cause categories:

- Cross-layer contract mismatch: multiplexed JSON-RPC transport wrapped in a serial session API.
- Implicit assumption: one missing response means the whole process is unusable.
- Test gap: no regression required a later request to finish before an earlier request timed out.

## Design decision

- Store the stdio transport as a cloneable `Arc<StdioConn>`.
- Keep a dedicated mutex only around physical stdin writes.
- Release the session lifecycle lock before awaiting a normal stdio response.
- Treat timeout as unknown execution outcome and do not retry.
- Reconnect only when the connection is actually dead or closed.
- Leave Streamable HTTP serialization unchanged because its mutable session-id lifecycle is a different contract.

## Automated regression

`lost_response_does_not_block_later_request` uses a fake stdio server that drops the response for `hang`, answers `two`, and records calls. It verifies:

- `two` completes before `hang` reaches its timeout;
- the timeout says the outcome is unknown and the call was not retried;
- both `hang` and `two` were sent exactly once.

## Real OfficeCLI live verification

A temporary local test exercised the full Kivio runtime path:

`AppState -> mcp_get_or_connect -> mcp_call_tool -> StdioConn -> officecli.exe mcp`

Configuration:

- OfficeCLI version: 1.0.135
- `OFFICECLI_RESIDENT_FLUSH=each`
- Kivio tool timeout: 4000 ms
- temporary PPTX with two blank slides
- partial batch: invalid `set` item used `parent` instead of `path`; a following `add textbox` item succeeded

Results:

- partial batch: 1 succeeded, 1 failed;
- first post-batch `set /slide[1]`: response lost and Kivio timed out at 4.001 s with `outcome is unknown and was not retried`;
- request B (`get /slide[2]`) was issued 200 ms later and completed successfully in 110.6 ms;
- request B therefore completed about 3.69 s before request A timed out.

This reproduces the real upstream fault while proving the fixed Kivio runtime no longer turns it into session-wide head-of-line blocking. The machine-local test was removed after execution so CI does not depend on OfficeCLI.

## External contract references reviewed

- MCP lifecycle specification: multiple requests may be in flight and JSON-RPC ids correlate responses.
- OpenAI Codex MCP connection manager: tool calls use a cloneable client rather than a session mutex held across response waiting.
- Claude Code documentation: documents MCP startup/tool timeout controls; it does not justify assuming automatic replay is safe.
