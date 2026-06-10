# Backend Development Guidelines

> Runtime and Tauri backend conventions for this project.

## Guidelines Index

| Guide | Description | Status |
|---|---|---|
| [Agent Runtime](./agent-runtime.md) | Chat agent loop, tool execution, and transcript contracts | Active |
| [HTTP Timeouts](./http-timeouts.md) | Provider HTTP client timeout contracts for normal requests, streaming SSE, downloads, and retries | Active |
| [Lens Chat Handoff](./lens-chat-handoff.md) | Lens-to-Chat transfer command, screenshot attachment handoff, and navigation events | Active |

## Pre-Development Checklist

- Read [Agent Runtime](./agent-runtime.md) before changing `src-tauri/src/chat/agent/**`, provider replay messages, or tool execution behavior.
- Read [HTTP Timeouts](./http-timeouts.md) before changing `src-tauri/src/api.rs`, provider adapters in `src-tauri/src/chat/model/**`, `send_with_retry`, `send_with_failover`, or any SSE streaming request/response loop.
- Read [Lens Chat Handoff](./lens-chat-handoff.md) before changing `lens_send_to_chat`, Lens screenshot cleanup, Chat conversation routing, or Lens-to-Chat attachment transfer behavior.

## Quality Check

- Run targeted Rust tests for the changed backend area.
- Run `cargo test --manifest-path src-tauri/Cargo.toml` when practical.
- For Chat agent changes, verify provider-compatible replay messages and deterministic tool result ordering.
- For Lens-to-Chat handoff changes, verify the screenshot survives Lens close cleanup and the Chat window navigates to the target conversation.
