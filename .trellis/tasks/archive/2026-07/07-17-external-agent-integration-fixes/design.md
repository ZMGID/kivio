# Technical Design

## Task Map

- `07-17-pi-epipe-shutdown`: 修正 Pi RPC 逻辑完成与进程输出收尾之间的生命周期边界。
- `07-17-opencode-custom-model-discovery`: 使用 OpenCode 原生命令发现模型，并让检测 cwd/cache 与项目隔离。

## Shared Invariants

- 外部 CLI 自己负责解释其协议与配置；Kivio 只消费稳定的协议事件或命令输出。
- fallback 只能用于兼容性兜底，不得覆盖已经获得的动态真值。
- 所有子进程探测都必须有超时、无控制台窗口并清理资源。

## Integration

两个子任务不共享代码修改，但共同依赖 `external_agents` 测试集。先合入 Pi 生命周期修复，再调整 OpenCode 检测 API 和缓存，最后运行同一轮完整检查以捕获交叉回归。

## Rollback

两个子任务可分别回滚。父任务不承载直接业务代码。
