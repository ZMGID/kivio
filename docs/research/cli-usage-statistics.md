# 本地 CLI 用量与统计概览

2026-11-14。接续 `usage-sqlite-storage.md`；本次增加原生 CLI 历史统计，不改变会话导入、绑定或续聊语义。ADR-0003 对通过 ACP 导入会话的约束保持不变；这里仅提取用量，不创建或导入对话。

## 调查依据

T3 Code [使用说明](https://github.com/pingdotgg/t3code/blob/main/docs/user/usage.md)将本地历史中的 token 和 API 等价费用与订阅额度分开。核对的读取器为 [Codex](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/provider/Drivers/codexUsage.ts)、[Claude](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/provider/Drivers/claudeUsage.ts)、[Pi](https://github.com/pingdotgg/t3code/blob/main/packages/provider-pi/src/server/usage.ts)、[Grok](https://github.com/pingdotgg/t3code/blob/main/packages/provider-grok/src/server/usage.ts)。本次以其数据来源和去重原则为依据，未引入 T3 的服务框架或依赖。

## 数据流

打开用量概览 / 刷新 → `usage_sync_cli` → 原生历史只读扫描 → SQLite 用量账本 → 现有统计查询。分页、来源筛选和搜索不触发扫描。扫描复用一个数据库连接；按文件长度与修改时间跳过未变的 transcript。OpenCode 使用只读连接，每次刷新读取数据库以涵盖 WAL 中的数据。

- Codex：默认及当前配置的 `CODEX_HOME` 下 sessions / archived_sessions，支持普通 JSONL 和 Zstandard 压缩历史，读取请求账本或旧 token_count 的单次用量，不累加累计快照。带 response_id 的账本优先，同一 turn 已先写入的快照会在账本到达时撤销；撤销 ID 持久化，旧副本不会重新计入。重复累计快照跳过，分叉复制的初始记录不计入。
- Claude Code：projects 下 assistant 的 usage。按消息 ID 与 request ID 去重，流式部分字段按通道取最大值；缓存读取、写入与普通输入先分开，再统一成包含缓存的输入总量。
- Pi：sessions 下 assistant、独立 usage、工具/总结用量；保留 CLI 上报费用，按条目 ID、时间和模型去重。
- Grok：仅读取 updates.jsonl 的 turn_completed，优先每个模型的 usage，costUsdTicks 按 10^10 换算美元；同一 prompt/model 保留首条完成记录，避免重复的放大记录覆盖原始费用。
- Kimi：读取 wire.jsonl 中 usageScope=turn 的 usage.record；不拿上下文快照当历史消耗。去重使用会话目录 ID、agent 名和事件内容，不依赖绝对路径或行号。
- OpenCode：读取 message 表的 assistant data，保留已上报费用。不支持的数据库结构显示错误，不伪装成零用量。

原生文件不修改，提示词、回复和工具正文不写入统计库。已同步的历史在源文件移除后仍留在账本；相同请求再次扫描或从复制文件出现时更新，不重复累计。清空账本同时保存截止时间，刷新不重新导入已清空的旧记录。JSONL 迁移备份和 CLI 原生历史保留。

## 费用与覆盖边界

有有效 CLI 上报费用时优先使用；否则使用 LiteLLM 的自动价格表（输入、输出、缓存读写、长上下文档位和原生 Codex 记录的服务档位）。价格与更新时间保存在统计 SQLite 数据库，24 小时过期；手动刷新绕过日缓存，但至少间隔 60 秒。下载失败保留缓存并显示诊断，没有缓存的未知模型保持未计价。刷新后在事务中重新计算历史自动估算，保留 CLI 上报费用和显式用户价格。没有完整输入/输出单价的模型保留 token、标为未计价。API 估算不代表订阅账单或实际扣费。现有配置覆盖与默认目录都参与发现；不枚举任意未配置账号目录、远程机器或 WSL 私有目录。

尚不扫描 Cursor / Antigravity 等来源，不新接订阅额度 API。OpenCode 支持 message 和 session_message 表，读入模型引用并将独立 reasoning 计入输出，网关明确标记的免费模型保留零价。历史缺失不代表免费或零消耗。LiteLLM 来源：https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json 。现有模型库仍负责能力和上下文信息，用量自动计价不再依赖其人工维护价格。

## 页面

概览展示费用/Token 切换、来源占比、小时或日期趋势、Token 构成与模型明细；保留请求日志和分页。24 小时按小时聚合，另有今日、7/30/90 日和全部。未知费用用未计价标识，CLI 历史缺少耗时时显示未知。移除独立的年度热力图请求和饼图，所有展示服从当前查询。

## 验证

临时文件/数据库用例覆盖 Claude 重复及部分字段、Codex 账本与快照去重、Pi/Grok 费用单位、OpenCode 只读解析、损坏/未完成行、同步幂等、补全用量和清空后的防重新导入。前端用例覆盖过期查询、自动同步、费用说明、未计价提示、指标切换、来源过滤以及筛选不重复扫描。

本机核验纠正：此前发现的 259 个 Codex 原路径不存在条目，256 个有对应 .zst 文件，并非历史丢失。不得将文件数直接当成丢失会话数。

## 2026-11-14 本机差分核验

独立执行 T3 当前公开的 `codexUsage.ts` 和 `usagePricing.ts` 的解析/计价函数（临时 Node harness 替换宿主 Schema 依赖，保持用量校验），与 Rust 原生读取器导出的计费元数据比较。使用同一本机历史和价格表，统一 30 日窗口，并排除仍在写入的最近两分钟。四个 Codex 模型的 Token 总量和价格结果一致（浮点末位舍入除外）。该核验不是 T3 完整 UsageService 的端到端测试，不包含其保留缓存、环境汇总、用户映射或订阅限额。

本机 OpenCode 读取到 session_message 新表，Grok 的首次完成记录和 Pi 用量也已核对。Rust 忽略测试 `usage::native::tests::audit_real_cli_history` 可显式重跑：只读原生文件并写临时统计库；设置 `KIVIO_USAGE_AUDIT_OUTPUT` 可导出 Codex 计费元数据用于差分，不含对话正文。普通测试不读取用户目录、不联网。

费用源实现依据：[usagePricing.ts](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/usage/usagePricing.ts) 和 [UsageService.ts](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/usage/UsageService.ts)。自动定价与 Kivio 原模型库解耦；现有显式模型价格覆盖继续生效。
