# 修复 Pi 结束阶段 EPIPE

## Goal

Pi 正常完成一次 RPC 回合后，Kivio 不再因提前关闭 stdout 读取端导致 Pi 抛出 `write EPIPE`，聊天界面应显示正常完成结果。

## Background

- GitHub issue: https://github.com/ZMGID/kivio/issues/17
- `src-tauri/src/external_agents/session/pi_rpc.rs:411-414` 在收到 `agent_end` 后设置结束标记并关闭 stdin；下一轮立即退出循环并释放 stdout reader。
- `src-tauri/src/external_agents/run.rs:337-405` 随后等待进程，并把非零退出及 stderr 追加为失败，因此 Pi 的收尾 EPIPE 会覆盖已生成的有效结果。

## Requirements

- R1. `agent_end` 表示逻辑回合完成，但不表示 stdout 已完成物理收尾。
- R2. 收到 `agent_end` 后只关闭 stdin 一次，并继续 drain stdout，直到 Pi 按 RPC 约定 flush 后关闭输出。
- R3. drain 阶段继续执行现有取消检查，用户主动取消仍可终止异常挂起的子进程。
- R4. 没有收到 `agent_end` 的读取错误或进程失败仍按现有错误路径处理。
- R5. 不通过字符串特判或无条件忽略 `EPIPE` 掩盖真实故障。

## Acceptance Criteria

- [x] AC1. 测试子进程输出 `agent_end` 后延迟写入一条收尾消息并正常退出，Kivio 持续读取到 EOF。
- [x] AC2. 上述场景返回 `Ok(())`，不会诱发子进程 EPIPE。
- [x] AC3. agent_end 后等待 EOF 的阶段仍响应取消，并返回 `cancelled`。
- [x] AC4. stdout 读取错误在未完成时仍返回错误；既有的非 JSON 行忽略行为保持不变。
- [x] AC5. 本次修改文件格式检查、针对性 Rust 测试和全仓 Rust 测试通过。

## Out of Scope

- 修改 Pi 上游 `output-guard.js`。
- 将 Pi RPC 改造成跨回合持久会话。
