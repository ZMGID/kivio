# Implementation Plan

1. 重构 `run_pi_rpc_session` 的结束状态，区分逻辑完成与 stdout EOF。
2. 收到 `agent_end` 后关闭 stdin，并按 Pi RPC 约定继续 drain 到 EOF；保留取消检查。
3. 增加 agent_end 后延迟尾写的回归测试，以及取消/错误保护测试。
4. 运行 `cargo fmt --check` 和 external_agents 相关测试。
5. 检查外层非零退出/stderr 规则，确认正常收尾不再被标记失败。

## Risk Points

- `src-tauri/src/external_agents/session/pi_rpc.rs`
- `src-tauri/src/external_agents/run.rs`（原则上只验证，非必要不改）

## Validation

- `cargo test --manifest-path src-tauri/Cargo.toml external_agents::session::pi_rpc`
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
