# Technical Design

## Probe Order

OpenCode 使用专用探测链：

1. 运行原生 `models` 命令，使用 OpenCode 自己的配置加载器获得模型真值。
2. 原生命令不可用时运行现有 ACP `session/new` 探测。
3. 两者都失败时由 `detect_single_agent` 使用 `OPENCODE_MODELS` 静态 fallback。

其他 ACP Agent 保持现有路径，不共享 OpenCode 特有的 CLI 假设。

## Parsing Contract

- 每个有效 stdout 行视为完整模型 ID。
- 只接受包含非空 provider 和 model 两部分的 `provider/model`。
- 保留模型 ID 大小写与深层路径，按完整 ID 去重。
- 结果前置统一 `default` 选项。

## CWD and Configuration

全局配置由 OpenCode 无条件加载；项目配置依赖探测命令 cwd。

- `chat_detect_external_agents` 接受可选 `conversationId` 并通过 `resolve_effective_cwd` 获取项目根或会话工作区；设置页缺少会话时使用进程当前目录。
- `detect_all_agents` / `detect_single_agent` 接受显式 cwd，所有模型 probe 都在该目录运行。
- `run_external_cli_reply` 先解析有效 cwd，再执行单 Agent 检测，确保启动前验证与当前项目一致。
- 前端 `RuntimePicker`、`ExternalModelSelector` 和 `PermissionPicker` 从 `Chat` 接收当前会话 ID，并传给检测 API。

## Cache Contract

- 全量检测缓存由单个全局值改为 `cwd -> (timestamp, agents)`。
- Agent 模型缓存使用 `agent_id:cwd` 键；上下文估算读取时通过 conversation 解析相同 cwd。
- 设置页和无会话调用使用稳定的当前目录键。
- `forceRefresh` 仅跳过当前 cwd 的缓存，不清空其他项目。

## Compatibility

- 旧版 OpenCode 没有 `models` 命令或命令失败时，ACP 与静态 fallback 保持兼容。
- 不改变其他 ACP Agent 的模型发现。
- API 的 `conversationId` 为可选参数，现有设置页调用保持兼容。

## Rollback

专用 probe 可独立移除，恢复现有 ACP-first 行为。
