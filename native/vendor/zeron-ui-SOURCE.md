# Zeron UI application source

Source: https://github.com/zeronsh/zeron/tree/c04487045e5f934818ab21bf5b7b7fbed16ef3d6/crates/ui/src

MIT copyright notice: [zeron-ui-LICENSE](zeron-ui-LICENSE).

- `native/src/transcript/details.rs`: fork divider adapted from `transcript.rs::fork_marker`; tool grouping follows `render_tool_group` interaction, implemented with Kit buttons and Kivio `ToolCallRecord`.
- `native/src/terminal/`: emulator and custom terminal element adapted from `terminal/emulator.rs` and `terminal/view.rs`; Kit palette and retained panel replace Zeron theme/daemon ownership. Detailed license in that directory.
- `native/src/shell/search.rs`: command/conversation search composition informed by `shell/command_palette.rs`; Kivio history search and Kit dialog remain authoritative. This is an interaction adaptation, not a claim that Zeron's full search module was copied.

Actual dependency API: GPUI Kit 0.8.0, gpui-pre 0.3.9. No Zeron GPUI fork, RPC or backend is linked.

- `native/src/shell/history.rs`: `NavHistory` copied from `shell.rs`, specialized to Kivio conversation IDs; unused route operations removed. Deleted conversations are skipped by the shell without recreating records.
- `native/src/shell/header.rs`: `render_titlebar_cluster` ported to Kit Buttons (sidebar, Back/Forward, new conversation). Kivio runtime selection and conversation actions remain connected to the existing shell owner.
- `native/src/preview.rs`: expanded composer pill and action-row composition ported from `composer.rs`; Kit Textarea replaces Zeron's editor, Kivio attachments/provider bindings replace its RPC and picker state. The original Kivio sending/cancellation/persistence owners remain in use. Wallpaper, cloud/device controls, browser surfaces and voice dictation are not imported.
- `native/src/shell/navigation.rs`: stacked disclosure/list/footer composition from `shell.rs::render_chat_sidebar`; populated by Kivio recent conversations, sets and projects. Kit buttons replace Zeron-specific widgets; Kivio manual order, pinning, drag/drop and group commands remain owned by their existing implementations.
