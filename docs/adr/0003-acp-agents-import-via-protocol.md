# ACP 代理走协议枚举与重放，不解析它们的私有存储

opencode 和 kimi 在 Kivio 里是 ACP 代理。导入它们的会话时，用 ACP 的 `session/list`（枚举）和 `session/load`（绑定/续聊），**不去读** `opencode.db` / `session_index.jsonl` 来做枚举。opencode 的 `session/load` 还会把历史**重放**成正常的 `session/update` 流——Kivio 的 `acp.rs` 本来就会渲染它，导入历史的保真度自动等同于实时聊天。

## 本机实测的能力矩阵（2026-08-01）

| 代理 | `session/list` | `session/load` | 历史重放 |
| --- | --- | --- | --- |
| opencode | ✅ 19 条，按 `cwd` 过滤正确 | ✅ | ✅ 15 条 `session/update`（`user_message_chunk` / `agent_thought_chunk` / `tool_call`） |
| kimi | ✅ 2 条 | ✅ | ❌ 只推一条 `available_commands_update` |
| gemini | ❌ `Method not found (-32601)` | ❌ `loadSession=false` | — |
| cursor | 未安装，未验证 | — | — |

**gemini 因此不支持导入**：Kivio 驱动 gemini 走的就是 ACP，`loadSession=false` 意味着导入后无法续聊，与 [ADR-0001](./0001-imported-cli-conversations-stay-on-their-cli.md)「导入必须能续聊」直接冲突。这是本机 gemini CLI 版本的能力；将来若它实现了 ACP 会话方法，可原样加回。

**当时的 kimi 需要额外的历史来源**：上述版本的 `session/load` 能绑定但不重放，当时计划从本地历史补齐显示快照，枚举仍走 `session/list`。

2026-11-07 核验官方 Kimi Code 源码 `eb2035f3cad57cc12d4d5082ec626f7f0ea5f651` 后更新此判断。[`loadSession`](https://github.com/MoonshotAI/kimi-code/blob/eb2035f3cad57cc12d4d5082ec626f7f0ea5f651/packages/acp-server/src/server.ts#L297) 在返回前等待 `replayHistory`，[重放实现](https://github.com/MoonshotAI/kimi-code/blob/eb2035f3cad57cc12d4d5082ec626f7f0ea5f651/packages/acp-server/src/replay.ts) 把主代理历史转换为 ACP `session/update`。Kivio 因此复用 ACP 重放，不新增私有文件解析器。加载后没有可显示历史时拒绝导入，避免把有原生上下文的会话保存为空快照。此处是源码核验，本机未安装 Kimi，未证明所有已发布版本均支持重放。

## Considered Options

**直接解析各自的本地存储做枚举。** 否决：格式各异（SQLite、jsonl 索引、按 cwd 哈希分目录的 JSON），而 `session/list` 一个方法就覆盖了，还顺带给出 agent 自己认可的 `sessionId`——那才是 `session/load` 会接受的 id，从文件里刨出来的不一定是。读文件确实更快，但这是导入列表的一次性开销，不在热路径上。

## Consequences

- 列会话要先起进程、握一次 ACP，比读文件慢。
- 依赖 agent 声明 `loadSession` 且实现 `session/list`；两者缺一即退回"不支持导入"，与 paseo 把会话枚举做成 provider 可选能力的做法一致。
- **`session/list` 必须带 `cwd` 参数**。不带的话 agent 返回的是全局最近的会话，分页 `limit` 会在够到当前目录的会话之前就截断——本来就要按项目根过滤，正好在源头传。
- 仍需读文件的：**claude**（`~/.claude/projects/<编码 cwd>/<uuid>.jsonl`）、**grok**（`~/.grok/sessions/<百分号编码 cwd>/<uuid>/`）、**codex**（若 CodexAppServer 协议没有等价方法，则读 `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`）。

## claude 的目录编码规则（易错，写下来）

`~/.claude/projects/` 下的目录名 = 规范化后的绝对路径，**每一个非字母数字字符**（`[^a-zA-Z0-9]`）替换成 `-`——空格、中文、盘符冒号、分隔符、点、下划线全算。若结果超过 **200** 字符，截断到 200 再追加一个哈希后缀。

编码**有损**，反解不回去。按项目根正向算一次即可精确定位，不要试图从目录名还原路径。

cwd 比对要 **realpath-aware**（先解符号链接 / junction 再比），直接比字符串会在链接路径上失配。
