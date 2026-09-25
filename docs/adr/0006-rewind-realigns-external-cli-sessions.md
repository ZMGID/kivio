---
status: accepted
---

# 「回到这里」同步回退外部 CLI 的原生会话

外部 CLI 对话的历史真身在 CLI 自己的原生会话里，Kivio 那份只用于显示（见 [ADR-0001](./0001-imported-cli-conversations-stay-on-their-cli.md)、[ADR-0002](./0002-imported-history-is-a-snapshot.md)）。「回到这里」原先只截 Kivio 这份，下一轮 resume 时 CLI 仍带着被删掉的轮次：用户看到的是 1、2，模型记得 1 到 7。

现在回退分两步：

1. 确认目标是用户提问后、截断之前，在 `external-agent-sessions/rewound-<对话 id>.marker` 记一笔「待回退」。记不下就整体失败，不截断；被拒绝的回退不留标记，以免补历史的 CLI 无谓地换掉会话。
2. 下一轮普通发送（斜杠命令除外）先把原生历史拉回可见历史，再发这条消息；这一轮完整成功才清除标记。Pi / Claude 的原生回退那一轮出错时，标记改为「补历史」，下一轮不再重试同一种原生回退（例如 CLI 去掉了所依赖的参数，重试只会一直失败）。

各 CLI 的拉回方式：

| CLI | 方式 |
|---|---|
| Pi | 原生 `fork`，与「重新生成」同一条路径 |
| Claude | 在转录链上找到第一条被删提问之前的条目，`--resume <旧 id> --resume-session-at <条目> --fork-session --session-id <新 id>` 续接一个截断的分叉；原转录不动 |
| Codex | 接续线程后 `thread/turns/list` 对齐，再 `thread/revert` 到第一条被删提问之前 |
| 其余（ACP 系、dsh、antigravity） | 丢弃原生会话绑定，开新会话，把剩下的可见历史整理成一段文字随首条消息发出一次 |

Claude 转录读不到、对不上（例如 CLI 压缩过上下文），或 Codex 不支持上述方法、对不上时，同样退到「新会话 + 补历史」。对齐一律按用户提问的顺序与文本前缀匹配，不按位置盲删；对不上就不裁剪原生会话。

## Considered Options

- **只开新会话、不补历史**：否决。回退点之前的内容 CLI 也全忘了，和「回到这里」的语义不符。
- **在这些 CLI 的对话里隐藏按钮**：否决。用户选择了保留功能。
- **直接改写 CLI 的转录文件**：否决。Kivio 从不写 CLI 自己的数据，用户在终端里还要能 resume 原会话。

## Consequences

- 「补历史」是「不重放历史」原则的一次性例外：只在回退后的第一轮，只带可见的文字（工具调用只剩前后文字），上限约 6 万字符，保留最新的部分。
- Claude 的做法依赖隐藏参数 `--resume-session-at`，在 claude 2.1.282 上实测：`system/init` 报告的是 `--session-id` 指定的新 id，新转录只含保留的前缀，原转录不变。CLI 若去掉该参数，那一轮会报错。
- Codex 在 0.157.0 上实测：`thread/turns/list` 的 `summary` 视图带 `userMessage`，`thread/revert` 后重新 `thread/resume` 仍只剩保留的轮次。更早的版本若没有这两个方法，自动退到补历史。
- Codex 的 `thread/revert` 只改写线程历史，不撤销文件改动；「回到这里」对其他 CLI 同样不撤销文件改动。
- 删除单条消息仍不同步到 CLI，不在本决定范围内。
