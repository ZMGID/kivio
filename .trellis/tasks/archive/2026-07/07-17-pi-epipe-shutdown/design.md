# Technical Design

## Boundary

改动限定在 Pi RPC 单回合运行器及其测试。外层 `run_external_cli_reply` 继续负责等待子进程、收集 stderr 和决定最终状态。

## Lifecycle

1. 写入 prompt。
2. 正常解析 stdout RPC 行。
3. 收到 `agent_end` 后记录 `agent_ended = true`，关闭 stdin。
4. 继续读取 stdout，直到 EOF；忽略已经完成后的非协议收尾行，但保持管道打开。
5. drain 期间保留每 200ms 的取消检查；若 Pi 异常挂起，用户取消仍会终止子进程。

Pi 上游 `runRpcMode` 明确监听 stdin `end`，执行 runtime dispose、`flushRawStdout()`，然后退出。因此关闭 stdin 后等待 stdout EOF 是协议规定的正常关机路径，不需要猜测固定 sleep 或按 `EPIPE` 文本兜底。

## Error Contract

- `agent_end` 前的 stdout 读取错误：返回错误。
- 用户取消（包括 agent_end 后 drain 阶段）：终止子进程并返回 `cancelled`。
- `agent_end` 后的纯协议解析噪声：继续 drain。
- 不在 Kivio 侧按错误文本忽略 `EPIPE`。

## Test Strategy

优先新增真实子进程管道测试：fixture 在收到 prompt 后输出 `agent_end`，短暂延迟，再输出尾行并退出。测试必须证明 reader 未提前释放。若跨平台 shell fixture 不稳定，则把循环抽成可注入 AsyncRead/AsyncWrite 的内部函数进行确定性测试。

## Rollback

改动集中在 `run_pi_rpc_session`；回滚不会影响其他 Agent 协议。
