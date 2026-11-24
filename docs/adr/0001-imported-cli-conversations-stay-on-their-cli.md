# 导入的 CLI 对话钉死在原 CLI 和原工作目录上

导入的对话由**原 CLI** 在**原工作目录**续聊。Kivio 保存用于显示的历史快照。发送新消息时，原 CLI 按原生会话 ID 或文件路径恢复自己的历史。

## Considered Options

**把历史翻译成 Kivio 的 `model_messages`，之后可用任意模型续聊。** 否决：要给每个 CLI 写工具名反向映射（claude 的 `Read`/`Edit`/`Bash` → `read_file`/`edit_file`/`run_command`），给每条 tool_result 编能过校验的 id，而 hook 注入、sidechain 子 agent、CLI 私有工具映射不上只能丢。丢完之后模型看到的历史和用户在界面上看到的不是同一份。

**对能跨目录 resume 的 CLI 放宽工作目录约束。** 否决：本机实测 codex / grok / opencode 确实能跨目录续上，但 Kivio 续聊时的 cwd 取自 `resolve_effective_cwd`（项目根）。会话续上了而工作目录不对，模型就会在错误的目录里读写文件——静默、无报错，比"找不到会话"危险得多。

## Consequences

- 导入是**项目内的动作**：只列出工作目录等于当前项目根的那些原生会话。
- 当前可导入 Claude Code、Codex CLI、Grok CLI、Kimi CLI、OpenCode、Pi、DeepSeek Harness 和 Antigravity CLI 的会话。Kimi、OpenCode 和 Antigravity CLI 通过 ACP 枚举会话并重放历史，见 [ADR-0003](0003-acp-agents-import-via-protocol.md)。Pi 读取 JSONL 的活动分支，续聊前核对实际打开的文件和会话 ID。DeepSeek Harness 通过官方 SDK 读取共享会话存储，使用原生 ID 续聊。
- 原工作目录不存在时（换机器、目录已删）不能导入，需要在导入前就明说，而不是让用户导进来之后每轮踩一次上下文重置。

## 本机实测依据（2026-08-01）

锁工作目录，必须在原目录才能续：**claude**（异目录报 `No conversation found with session ID`，回原目录同一 id 正常续上）、**kimi**（报 `was created under a different directory` 并打印 `cd` 命令）、**gemini**（`--list-sessions` 换目录即为空）。

不锁，跨目录也能续：**codex**、**grok**（会打印 `originally in <原目录>`）、**opencode**。
