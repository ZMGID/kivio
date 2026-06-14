# Make MCP Agent Tool Limits Unlimited

## Goal

MCP tool execution settings should not impose a max tool-round limit or result truncation limit for Agent runs. The settings UI should reflect that these two values are unlimited instead of asking users to enter ineffective numeric limits.

## What I Already Know

- The MCP settings page currently shows numeric inputs for max tool rounds, tool timeout, and result truncation characters.
- The user specifically wants max tool rounds and result truncation characters to be unlimited.
- Tool timeout remains a real runtime guard and should stay configurable.
- Current Rust settings clamp `max_tool_rounds` and `max_tool_output_chars`, so backend defaults and sanitization must change alongside the UI.

## Requirements

- Treat Chat/MCP `max_tool_rounds` as unlimited for Agent tool loops.
- Treat Chat/MCP `max_tool_output_chars` as unlimited, preserving full tool output for model replay and stored previews where that setting is used.
- Remove or replace the MCP settings UI controls for these two numeric limits so users are not prompted to configure them.
- Keep tool timeout configuration unchanged.
- Preserve compatibility with older saved settings that contain numeric values.

## Acceptance Criteria

- [ ] Agent runtime no longer stops because `max_tool_rounds` is reached.
- [ ] Tool results are no longer truncated by `max_tool_output_chars`.
- [ ] MCP settings page does not expose numeric inputs for max tool rounds or result truncation characters.
- [ ] Existing saved settings containing old numeric values load without errors.
- [ ] Type-check and targeted Rust tests pass where practical.

## Out of Scope

- Removing unrelated truncation used for chat summaries, prompts, or UI-only previews outside MCP/Agent tool result handling.
- Changing tool timeout behavior.
- Reworking MCP approval/server configuration.

## Technical Notes

- Likely frontend file: `src/settings/SettingsShell.tsx`.
- Likely backend files: `src-tauri/src/settings.rs`, `src-tauri/src/chat/agent/loop_.rs`, `src-tauri/src/chat/agent/execute.rs`.
