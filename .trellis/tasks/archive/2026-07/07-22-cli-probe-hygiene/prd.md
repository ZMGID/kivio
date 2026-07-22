# PRD — 斜杠探测残渣治理与探测副作用清查

来源：磁盘级审计 `.trellis/tasks/archive/2026-07/07-20-external-cli-overhaul/research/session-files-audit.md`——kimi 单会话 12 分钟堆 25 个空壳会话（斜杠探测每次发 2 个 session/new + 空结果不缓存无限重探）；cursor 历史遗留 4065 个空目录同源。

## Goal

Kivio 发起的任何探测不再在 CLI 侧留下随使用无界增长的残渣；顺带清查其余探测路径的同类副作用。

## Requirements

### R1 空结果负缓存（主修，消掉 25 倍放大）
`slash.rs::list_external_cli_slash_commands`：探测结果为空时也写缓存（负缓存，TTL 与模型探测 fallback 一致 30s 或稍长；force 语义如有则绕过）。切会话/切 agent 不再每次重探。

### R2 斜杠探测 cwd 对齐全局 scope
斜杠探测 cwd 从 `resolve_effective_cwd`（每会话）改为 `resolve_detection_cwd`（`__global__`）：残渣不再散落各会话工作区，缓存键全 App 共享（缓存 key 随之统一）。斜杠命令列表与 cwd 无关（CLI 全局命令），语义安全。

### R3 去掉斜杠探测的冗余模型 probe
`slash.rs` 的 `detect_single_agent` 调用改为 `detect_availability_single`（或直接 `resolve_binary`）——它只需要可用性，却连带跑了一遍完整 `probe_models`（又一个 session/new + 最多 15-25s）。这也与 spec 契约第 9 条"回复路径零探测"的精神一致（审计已备注该残留）。

### R4 其余探测路径清查（检查项，逐个确认无同类问题）
- `detect_acp_models`（模型探测）：已落 `__global__` 且有缓存+负缓存——确认无重复 session/new（一次探测一个）。
- `detect_claude_models` 的 `probe_claude_init`：会不会在 `~/.claude/projects/__global__ 对应 slug/` 下堆探测会话文件？检查并评估（claude 每次 probe 是真实 stream-json 启动）。
- codex `debug models`、pi `--list-models`：子命令型，确认无会话副作用。
- grok `__global__` 下 15 个探测小会话：随缓存（300s/30s TTL）增长有界性评估——若无界仍需治理。
- availability 探测（`--version`/auth probe）：无会话语义，确认即可。

## 非目标

- 不清理已有存量残渣（kimi 25 个、cursor 4065 个——属用户数据目录，列入报告让用户自行决定；可给一句手动清理命令）。
- 不改 CLI 侧行为。

## Acceptance Criteria

- [ ] 单测：空斜杠结果写入负缓存、TTL 内不再重探；斜杠探测 cwd 为全局 scope；斜杠探测不触发 probe_models。
- [ ] 实测：新会话在 grok/pi/kimi 间来回切换 10 次 → kimi 会话目录**零新增**空壳（`ls ~/.kimi-code/sessions/ | wc -l` 前后对比）。
- [ ] R4 清查结论写入本任务 research/ 或直接更新审计报告。
- [ ] `cargo test --lib` + 前端三命令全绿。
