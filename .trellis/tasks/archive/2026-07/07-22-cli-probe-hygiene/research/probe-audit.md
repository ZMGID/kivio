# R4 探测副作用清查结论

- 日期：2026-07-22
- 方法：只读代码走查 + 磁盘取证（`ls`/`find`，未改动任何 CLI 侧文件）
- 关联主修：R1 空结果负缓存 / R2 斜杠探测 cwd 对齐 `__global__` / R3 去掉斜杠探测冗余 `probe_models`

## 一句话结论

主漏（斜杠探测）已由 R1-R3 治理：从「每次切会话 2 个 session/new、散落各会话工作区、永不缓存」降为「≤1 个 session/new、统一落 `__global__`、30s 负缓存兜底」。其余探测路径经清查**均已有界**，无需额外治理——差异只在「CLI 是否 eager-persist」，而发起次数已由缓存夹紧。

## 逐项清查

### 1. `detect_acp_models`（模型探测，grok/kimi/cursor/opencode/gemini） — ✅ 有界
- cwd 走 `resolve_detection_cwd` → `__global__`（`commands.rs:70`），缓存键全 App 共享。
- 缓存 source-aware：probed 300s / fallback 30s 负缓存（`get_cached_external_agent_models`），single-flight（`model_probe_lock_for`）。
- 一次探测发**一次** `session/new`（`detect_acp_models`，acp.rs），无重复。
- 增长有界：正常缓存命中期内零新增；持续失败时最多每 30s 一个（fallback TTL）。**无需治理。**

### 2. `detect_claude_models` / `probe_claude_init`（claude ClaudeInit 探测） — ✅ 有界（CLI 惰性落盘）
- 每次探测用新 `new_session_id`（uuid，claude_init.rs:144），收到 `system/init` 即 `start_kill`，**从不跑对话轮**。
- 磁盘取证：`~/.claude/projects/<__global__ slug>/` 下**仅 1 个 `.jsonl`**（`d4950ec0…`，31KB 真实会话），尽管期间跑过多次探测。
- 结论：**claude Code 惰性落盘**——握手即被 kill 的探测**不写 transcript**（与 grok 同类，和 kimi 的 eager 落盘相反）。claude 探测残渣≈0。**无需治理。**
- R2 副作用：斜杠探测 cwd 从每会话工作区改为 `__global__` 后，claude slash 探测落到同一 global slug，因惰性落盘同样不堆积。R3 又移除了 slash 路径里对 `detect_single_agent`→`detect_claude_models` 的冗余调用，slash 探测不再触发 claude 模型探测。

### 3. codex `debug models` / pi `--list-models`（子命令型探测） — ✅ 无会话副作用
- codex：`probe_models` 走 `list_models_args`（`codex debug models`，输出 JSON 解析），纯子命令 `output()`，**无 `session/new`**。当前模型另读 `~/.codex/config.toml` 顶层键（无进程）。
- pi：`--list-models`（`models_from_stderr` 分支），纯子命令 `output()`。磁盘取证：`~/.pi` 下**无 `__global__` 会话目录**。
- 结论：两者均为一次性子进程、无会话语义。**无需治理。**

### 4. grok `__global__` 探测会话增长有界性 — 🟡 有界（可接受）
- 磁盘取证：`~/.grok/sessions/…__global__/` 有 **15 个** session 目录（Jul 20 + Jul 22 测试期累积），每个是探测 query 的小真实会话。
- grok CLI 惰性落盘：真实对话按 cwd 复用单会话、不随轮次膨胀（audit 检查项 3 已证）。`__global__` 的 15 个来自模型探测每次 `session/new`。
- 增长有界性：受 300s/30s 缓存 TTL 夹紧——正常使用下缓存命中零新增；最坏情况（反复 force 刷新或持续 fallback）约每 30s ≤1 个。属「测试期高频探测」的累积，非无界泄漏。**可择机手动清理，代码侧无需治理。**

### 5. availability 探测（`--version` / auth probe） — ✅ 无会话语义
- `detect_availability_single` = `resolve_binary` + `probe_version` + `probe_auth`，均为 `--version`/auth 子命令 `output()`，**不发 `session/new`**，cwd 无关。
- R3 后斜杠探测的可用性检查改用它（替代跑满 `probe_models` 的 `detect_single_agent`）——正是利用它「零会话副作用」。**无需治理。**

## 主漏治理前后对比（斜杠探测，kimi 为放大最严重者）

| 维度 | 修前 | 修后（R1-R3） |
|---|---|---|
| 每次探测 `session/new` 数 | 2（detect_single_agent 冗余 probe + detect_acp_commands） | 1（只 detect_acp_commands） |
| 空结果缓存 | 无（每次 useEffect 重探） | 30s 负缓存 |
| cwd / 缓存键 | 每会话独立 workspace（新会话必冷探） | `__global__`，全 App 共享 |
| 残渣落点 | 散落各会话工作区（污染 kimi 会话历史） | 统一 `__global__` |
| 12 分钟切换 10 次的空壳量 | ~25（实测） | ≤1（30s TTL 内命中，且集中在 `__global__`） |

## 存量残渣（不清理，交用户）

- kimi：`~/.kimi-code/sessions/wd_conv_*/` 历史空壳（单场 25 个）+ `wd___global___*/` 现 12 个。手动清理：
  `find ~/.kimi-code/sessions -type d -name "session_*" -empty` 或按 `state.json` 的 `users==0` 甄别后删除。
- cursor：`~/.cursor/acp-sessions/` 4065 个 76B `meta.json` 空壳（历史遗留，非本次）。手动清理：
  `rm -rf ~/.cursor/acp-sessions/*`（用户自担；均为空探测残留，无对话内容）。
- grok：`__global__` 15 个 + 若干 `/tmp/grok-test` 等测试期 cwd 残留，规模可控。

> 注：以上为**用户数据目录**，Kivio 不主动删除（非目标）。CLI 侧 eager/lazy 落盘行为不改（非目标）。
