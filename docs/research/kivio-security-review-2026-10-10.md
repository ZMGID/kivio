# Kivio 安全与可靠性问题核验

初次核验日期：2026-10-10。基线：`e44f3fa788905584f994bd013619ac1dba617df6` 加当前未提交改动（含上一轮 thinking/SSE/rmcp/取消修复）。以下核验表与详细证据记录修复前的快照。初次核验为只读；用户随后授权实施，2026-10-11 的修复状态见下一节。用户提供的 `/workspace/kivio-review-report.md` 在本机不存在，按消息列出的 17 项核验。

判断中的“成立”表示当前源码和相关契约能够支持问题；不表示已成功利用。Windows 分支和官方 Anthropic 均未在真实环境验收。用户明确要求凭据使用 JSON，不采用全部迁入钥匙串的方案。

## 2026-10-11 已实施的修复

- Windows 文件编辑不先删原文件；失败保留同步后的临时新内容并给出恢复路径，瞬时失败有界重试。
- Anthropic 原生 thinking/redacted/citations/server-tool 块按顺序回放；共享工具结果配对适用于 OpenAI/Gemini，Anthropic 客户端 ID 统一映射，跳过空 text，服务端 `pause_turn` 最多续轮 8 次。计费累计与末请求上下文用量分开。
- 新安装、缺失或非法审批值默认需要确认，保留已有有效 auto（旧默认与用户选择无法区分）；多答模式移除强制 auto，无法交互确认时跳过需要确认的操作。工具输出信任规则不依赖用户自定义 system prompt。
- 普通设置导入禁用外来 MCP/hooks，并保留本地 CLI 配置；可信恢复作为独立显式动作。完整备份保留恢复凭据，分享导出去除凭据、执行配置与 URL 认证参数。
- Provider OAuth 统一为私有 JSON 文件，成功落盘后才迁移删除旧 keyring；损坏 JSON 不回退旧条目。共享凭据 ID 的备份恢复保留单一 refresh authority，不覆盖现有账户文件。设置写入收敛到同一原子 JSON 负责人。
- shell/fetch/后台日志页面有界读取；Mac CLI/probe/MCP 进程生命周期清理组，Windows MCP 使用 JobObject 并保留无窗口创建 flags。
- PDF 库及安全依赖定向升级，审计漏洞命中 18→0；CI 补充 fmt/clippy/audit 和 Windows Rust job。
- 主页面 CSP 与隔离 HTML 预览、逐跳 HTTPS 检查、内网 URL 禁止第三方 reader、草稿原子重置/启动恢复屏障、输出额度预留及图片预算均已实施。

最终验证：Rust lib **2846 passed / 0 failed / 55 ignored**；前端 **328 files / 2736 tests passed**。Mac `cargo clippy --all-targets`、全仓 fmt、协议生成物、lint、架构、UI 生产构建、图标与打包检查均通过。Rust binary/三 integration 目标也通过，包括隔离目录的真实设置/会话读写与 AppKit 组件生命周期。完整 TS 检查仍有 **26 项既有 ES lib 声明错误**（`.at` / `replaceAll`）；Clippy 存量 warnings 保留。cargo audit 漏洞命中为 **0**，另有 11 unmaintained / 7 unsound / 1 yanked warnings，未忽略这些警告。首次全量 Rust 发现六个失败后，保留有效断言修正不完整协议/授权调度夹具，并恢复实际 bash 日志/整行尾部行为，再全量通过。

保留范围边界：完整 runtime `Vec<Value>` 类型迁移未做；会话已有 revision/CAS，不新增版本字段；journal 生命周期复活假说未复现。内网直接访问仍允许，因此不是全面 SSRF/DNS-rebinding 防护。Windows CLI 正常退出后遗留孙进程仍需要独立 JobObject 验证，不能用 Mac 组测试证明。完整原生应用、官方 Anthropic 和真实 Windows 验收尚缺。修复已本地提交为 `fceb0d68`，未 push；已保留其他并行 dirty 改动。

### 提交后独立审查（2026-10-11）

从 `fceb0d68` 创建独立工作树，未混入 Bot、原生窗口、工作区及前端工具链的并行改动。该提交 Rust lib **2841 passed / 0 failed / 55 ignored**；前端 **325 files / 2725 tests passed**；fmt、lint、TypeScript、UI 构建、架构、资源与打包检查通过；Rust audit 漏洞命中 **0**。上文 2846/2736 和 26 项 TS 错误来自此前混合工作树，不能作为独立提交的验证结果。

提交后审查确认两项遗漏，并追加最小修复：

- **P2，Unix MCP 正常断开遗留孙进程**：真实 rmcp `graceful_shutdown()` 返回成功后，leader 已退出但 `sleep` 孙进程仍运行。新增生命周期 owner，在正常 wait/try_wait 完成时立即清理所属进程组，Drop 兜底；真实 transport 回归保留 transport 并检查后代停止，Windows Job 路径保持原有实现。
- **P3，HTML 预览过期 URL 仍可访问**：仅创建新预览时清扫，旧 URL 超过一小时仍返回 200。新增请求时 TTL 校验和移除，返回空 404。新回归在原实现上已实际失败（200，预期 404）。

协议、审批、备份、凭据、输出与重定向其余分配范围未确认新的阻断缺陷。静态审查与本机回归仍不能代替官方 Anthropic、Windows 或完整原生应用验收。

补修后独立树 Rust lib **2843 passed / 0 failed / 55 ignored**，包含新增预览过期与 MCP 正常关闭孙进程回归；两项补丁均已由对应审查者复审通过。最终 `cargo clippy --locked --all-targets`、fmt 和协议生成物检查通过；保留现有 clippy warnings。

## 对用户 17 项的结论

| 原编号 | 项目 | 当前结论 |
| --- | --- | --- |
| 严重 1 | Windows 删除后 rename | 成立，P1；失败时两份内容均可能丢失。直接覆盖、有限重试并保留失败临时文件。 |
| 严重 2 | thinking 签名/原块丢失 | 原问题成立；当前工作区的上轮修复已覆盖 signed/redacted/client-tool 回放。官方实测仍缺，不把它重复列为未修。 |
| 严重 3 | 默认自动审批、提示注入 | 默认免审批与普通工具结果缺少统一政策成立；成功窃取密钥未利用验证。P1 组合风险，已有参数、启用、hook deny/ask 检查仍生效。 |
| 中等 1 | 明文凭据与 keyring 不一致 | 成立，但 JSON 是用户明确偏好；应考虑 provider OAuth 从 keyring 迁到 JSON，而非反向迁移。 |
| 中等 2 | 导入 MCP/hooks 可执行配置 | 条件成立，提升为 P1；导入函数不直接执行，后续连接/聊天会触发，且不全受工具审批控制。 |
| 中等 3 | OpenAI/Gemini 缺结果不修复 | 成立于已残缺历史；当前只重排现有结果，没有完整性修复。真实 HTTP 400 未复现，不能宣称正常历史必然失败。 |
| 中等 4 | `functions.x:0` 对 Anthropic 非法 | 原样传递成立；主会话补查官方 client-tool schema 确认 ID 只许字母数字下划线短横，点/冒号不符。具体 400 未实测。 |
| 中等 5 | shell/fetch 截断前全量读 | 成立，P2；后台 bash 写日志已避免该问题，但增量读取日志仍先读全文件。 |
| 中等 6 | Mac MCP/CLI 孙进程泄漏 | 部分路径成立，P2；本机受控进程实验验证反例。Antigravity、原生 bash、ACP terminal 已有组保护，不能一概而论。 |
| 中等 7 | PDF 栈溢出 | 成立，建议升为 P1 修复优先级；生产入口使用受影响 lopdf，大小限制与 catch_unwind 不能兜底。 |
| 中等 8 | 18 个依赖漏洞 | 实跑确认 18 命中；并非 18 个独立漏洞或全部可利用。兼容更新能修大部分，PDF/calamine 需升级上游约束。 |
| 中等 9 | CI 缺 clippy/fmt/audit/Windows | 缺口成立；“CI 只跑 cargo test”不准确，已有前端、协议、架构、打包门，Windows 发布构建也已存在。 |
| 小 1 | CSP 缺省 | 成立，P2 防御缺口；未据此证明 XSS/RCE。 |
| 小 2 | fetch 重定向不检查 | HTTPS 首跳约束可绕过，成立，P2；是否禁止私网是产品边界，若允许私网应避免 Jina 回退外传内部 URL。 |
| 小 3 | 空 assistant → 空 text | Anthropic 路径成立，P2 请求兼容问题；可能被官方拒绝，未在本仓实测。 |
| 小 4 | journal 非原子、会话无版本 | journal 超限截断重写成立，P2；“会话没有版本”不成立，已有 revision/锁/CAS。journal 缺 base revision 属另一个待复现恢复风险。 |
| 小 5 | token 低估、小窗压缩晚 | 条件成立；已有 usage 锚和小窗 15% headroom。剩余问题是预留未绑定真实输出额度、无锚图片/难分词文本估算不足。 |

取消的更正成立：等待流使用 `select!`，此前说“轮询导致无法打断流”过于笼统；上轮将 generation 失效等待改为通知，提高的是唤醒及时性，不能声称原实现完全不能取消。

另发现一个协议能力缺口：Anthropic server-tool 原生块未完整保存，`pause_turn` 被映射为普通 stop；这是与已修 client-tool thinking 回放不同的边界，见后文。

## 建议修复批次

1. Windows 写入失败保留旧文件与新临时内容，补 Windows 故障路径验证；PDF 安全解析库升级同列优先，避免继续接收恶意 PDF 导致进程中止。
2. 安全默认审批与不可信备份执行项信任；分别处理工具执行、MCP 启动、生命周期 hooks，提示标签不能代替权限边界。保留用户明确选择的 auto 和受控全量备份。
3. 有界 shell/fetch 输出、fetch 每跳 HTTPS/私网政策、MCP/CLI 进程关闭；各按原负责人小批修复。
4. 依赖按链定向更新并验证，解决现有 clippy/fmt 失败后补 PR macOS/Windows 门与 audit；不能用未测试的 377 包更新代替升级验收。
5. 公共工具结果配对、跨 provider ID 映射、空 Anthropic 块与 journal 原子重置；再处理 CSP、输出感知压缩、server-tool continuation。
6. 按全 JSON 偏好统一 provider OAuth 凭据存储和可恢复备份；不要把本地明文设计自动当作必须改用钥匙串的缺陷。完整备份应保留，分享用途另提供脱敏版本。

## 本机质量与依赖检查

环境：macOS arm64，Rust 1.99.0，cargo-audit 0.22.2（仅安装到临时目录）。

| 命令 | 结果 | 范围说明 |
| --- | --- | --- |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --message-format=json` | 失败，2 项不同的阻断错误 | `src-tauri/src/macos_ocr.rs:324` 的 `never_loop`；`src-tauri/src/automation/runner.rs:1079` 测试服务忽略读取字节数 `unused_io_amount`。两项不能直接当作新增安全漏洞。其余 323 项去重 warning 含编译器和 clippy 诊断。 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check` | 失败 | 输出 190 个格式差异位置，涵盖已有代码；没有自动格式化大工作区。 |
| `cargo-audit audit --file src-tauri/Cargo.lock --json` | 失败，18 项漏洞命中 | 14 个不同 advisory，9 个包、11 个包版本；另有 unmaintained 11、unsound 7、yanked 1 条 warning，不能合称为 18 个已验证可利用漏洞。 |
| `cargo update --manifest-path src-tauri/Cargo.toml --dry-run` | 成功解析 | 拟更新 377 个包，未改锁文件；说明兼容更新能够解决大部分命中，但不建议把全量升级未经测试直接落地。 |
| `git diff --check` | 通过 | 不替代格式、平台和安全验收。 |

RustSec 数据库提交：`7eebec69c352c7191b1f13eb95dd510eeca5d1de`，更新时间 2026-10-09。

### 18 项依赖命中

| 包（锁定版本） | Advisory | 已修复版本 |
| --- | --- | --- |
| bytes 1.11.0 | RUSTSEC-2026-0007 | >=1.11.1 |
| crossbeam-epoch 0.9.18 | RUSTSEC-2026-0204 | >=0.9.20 |
| h2 0.4.13 | RUSTSEC-2026-0258 | >=0.4.16 |
| lopdf 0.34.0 | RUSTSEC-2026-0187 | >=0.42.0 |
| quick-xml 0.30.0 / 0.31.0 / 0.38.4 | 每版本各命中 RUSTSEC-2026-0194、RUSTSEC-2026-0195，共 6 项 | >=0.41.0 |
| quinn-proto 0.11.13 | RUSTSEC-2026-0037、RUSTSEC-2026-0185 | >=0.11.15 可覆盖两项 |
| rustls 0.23.36 | RUSTSEC-2026-0285 | >=0.23.45 |
| rustls-webpki 0.103.9 | RUSTSEC-2026-0049、0098、0099、0104，共 4 项 | >=0.103.13 且 <0.104.0-alpha.1 可覆盖四项 |
| time 0.3.46 | RUSTSEC-2026-0009 | >=0.3.47 |

dry-run 选择了 bytes 1.12.1、crossbeam-epoch 0.9.21、h2 0.4.20、quinn-proto 0.11.19、rustls 0.23.45、rustls-webpki 0.103.15、time 0.3.55，并移除了 quick-xml 0.30.0/0.38.4，加入 0.41.0/0.42.0。但 `pdf-extract 0.7` 约束 `lopdf 0.34`，`calamine 0.26.1` 约束 `quick-xml 0.31`；这两条链需要升级上游依赖/API，普通兼容 `cargo update` 无法全部清零。还需按调用路径和平台评估可达性，尤其 quick-xml 0.30 属 Linux xcb 构建依赖。

### PDF：成立，建议提高优先级

`src-tauri/src/chat/knowledge_base/parse.rs:69` 在应用进程内调用 `pdf_extract::extract_text`；20 MiB 文件大小限制拦不住很小但极深嵌套的 PDF。当前 `Cargo.lock` 的 lopdf 为 0.34.0。官方 [RUSTSEC-2026-0187](https://rustsec.org/advisories/RUSTSEC-2026-0187.html) 明确确认 <=0.41.0 的无限递归会栈溢出并中止进程，`catch_unwind` 无法兜底，>=0.42.0 修复。本轮没有拿恶意 PDF 测试正在运行的应用；版本、生产解析入口和官方公告已构成明确证据。

### CI：缺口成立，“只跑 cargo test”描述不准确

`.github/workflows/ci.yml:26` 的质量任务只有 macOS runner，`:65` Rust 检查为 cargo test，没有 fmt/clippy/audit 或 Windows PR 检查。它同时运行前端 lint/typecheck/test、协议/架构、图标与打包检查，所以并非整个 CI 只跑 cargo test。`.github/workflows/release.yml:61` 已有 Windows 发布构建，缺的是 PR 阶段 Windows 质量门。新增 clippy/fmt 门前先解决上面的现有失败，不机械开启 `-D warnings` 阻断全部既有 warning。

## 逐项源码核验

其余条目的精确路径、成立条件与最小修复建议由本轮独立审查合并如下。

## 设置、权限与备份安全核验（只读，2026-10-10）

根目录：`/Users/zmair/ZM database/kivio`。本轮没有修改生产代码、运行导入、导出真实密钥或启动 MCP/hooks；结论来自当前源码。

## 1. 默认 auto 会跳过工具审批：成立，组合风险 P1

- `/Users/zmair/ZM database/kivio/src-tauri/src/settings.rs:1405` 默认 `auto`；`:2957` 一次性旧安装迁移启用 read/write/edit/run_command 并把旧默认权限改成 auto。
- `/Users/zmair/ZM database/kivio/src-tauri/src/chat/agent/execute.rs:300` 文件/shell：auto 不走会话 consent 或逐次审批；`:521` 其他工具：auto 直接 false，MCP 也不审批。
- 不是“没有任何 gate”：同文件 `:250` 校验 hook 修改后的参数，`:264` hook ask 即使 auto 仍询问，hook deny、工具启用、插件状态等仍检查。非 auto 的 MCP 必须审批，不信任服务自行声明的 readOnlyHint（`:528`）。
- 工作区不是沙箱：`/Users/zmair/ZM database/kivio/src-tauri/src/native_tools/mod.rs:74` 明确 default_directory 只是相对路径基准；绝对路径遵循 OS 权限。
- 最小建议：如产品希望安全默认，改新安装默认及迁移，保留用户显式 auto；不能把用户已选择 auto 当绕过漏洞。默认策略是产品决定，本轮不改。

## 2. 通用工具输出没有统一不可信政策：成立；成功注入/执行仅条件成立，P1 组合风险

- `/Users/zmair/ZM database/kivio/src-tauri/src/chat/agent/execute.rs:364` 成功结果截断后作为模型文本；`/Users/zmair/ZM database/kivio/src-tauri/src/chat/agent/rounds.rs:765` 仅 `{role:tool, tool_call_id, content}`，无通用 trust 标签/告警包裹。
- 检索 chat/native_tools/web_search 的 untrusted、prompt injection、不可信等没有找到普通 tool-result 的统一系统提示政策；role=tool 已区分角色，但不能据此断言模型绝不会听工具内指令。
- 有局部防护，不能说“全仓完全没有”：`/Users/zmair/ZM database/kivio/src-tauri/src/chat/memory.rs:620` 拒绝三种注入关键词写入记忆；`/Users/zmair/ZM database/kivio/src-tauri/src/chat/workflow_hooks.rs:166` 普通工具 hook 非 JSON stdout 是诊断，不作为模型指令。
- 恶意网页、文件或 MCP 返回诱导模型执行命令/泄露凭据的内容，加上默认 auto 与 OS 文件权限才形成危险链；本轮未做 exploit，不能宣布已验证任意执行。
- 最小建议：系统提示建立工具返回/检索内容是资料而非授权来源的统一政策；对外发数据/命令/写入增加实际权限 gate。提示标签只降低风险，不能替代执行边界。

## 3. API/MCP 凭据明文 JSON、provider OAuth 钥匙串：成立

- `/Users/zmair/ZM database/kivio/src-tauri/src/settings.rs:3269` API key 明确 JSON 策略；`:3433` 整体序列化到 settings Store。
- 同文件 `:1203` MCP env/header 明文策略，`:1247` ConnectorAuth 保存 access_token、refresh_token、client_secret。MCP 刷新会更新设置（`/Users/zmair/ZM database/kivio/src-tauri/src/mcp/manager.rs:435`）。
- `/Users/zmair/ZM database/kivio/src-tauri/src/provider_oauth.rs:208`、`:241` provider OAuth 存 Kivio.ModelOAuth keyring 的 manifest/chunks；设置只保留 credentialId，`:518` resolve 成临时 clone。不应宣称 provider OAuth token 也已在 settings.json。
- 用户明确偏好全 JSON；明文本地存储本身属既定设计，不建议反向搬全部凭据进 keyring。不同策略是偏好/可移植性问题 P2；备份拷贝暴露真实 token 需结合访问条件评估。
- 最小建议：将 provider OAuth 独立 JSON 凭据存储接入现有 id 与刷新生命周期，迁移已有 keyring entry，保持单一权威和原子写入；全备份包括必要凭据，分享导出另设脱敏用途；限制文件 OS 访问权限。

## 4. 设置备份包含真实凭据：成立；“静默泄密/自动上传”不成立

- `/Users/zmair/ZM database/kivio/src-tauri/src/commands.rs:292` 导出 sanitize 后整个 Settings，`:304` 无脱敏序列化，`:308` 原子写本地选定路径。sanitize 是规范化，不是 secret redaction。
- `/Users/zmair/ZM database/kivio/src/settings/SettingsShell.tsx:1089` 已明确“全部设置（含 API Key）与集”；`/Users/zmair/ZM database/kivio/src/settings/useSettingsBackupController.ts:30` 用户选本地文件后导出，没有自动上传。
- 若分享/同步备份给不可信主体则暴露 API key、MCP header/env/refresh/client secret（P1）；用户受控本地全备份与全JSON偏好一致。
- provider OAuth keyring token 不在备份中；换机器只有 credentialId 会失去登录态，须重新登录，不能声称导出保全所有 OAuth 凭据。
- 最小建议：保留全量本地备份并清楚标记敏感内容；额外分享用途脱敏，不以静默删密钥破坏恢复。provider OAuth 改JSON后明确同一备份契约。

## 5. 导入可执行配置：条件成立，P1；“导入函数直接执行 MCP/hooks”不成立

- `/Users/zmair/ZM database/kivio/src-tauri/src/commands.rs:314` 读取/解析 type/settings，然后 `:336` apply_settings；不去掉 enabled MCP/hook/auto。type= settings-backup 不是来源认证。
- `/Users/zmair/ZM database/kivio/src/settings/useSettingsBackupController.ts:34` 选文件后直接 import，没有可执行项预览；`/Users/zmair/ZM database/kivio/src-tauri/src/settings.rs:1486` sanitize_hooks 仅事件、类型、非空与超时，不审查 shell 内容。
- apply/commit 本身不启动 MCP 或 hooks。MCP 在聊天准备工具目录（`/Users/zmair/ZM database/kivio/src-tauri/src/chat/commands/tooling.rs:185` → `/Users/zmair/ZM database/kivio/src-tauri/src/mcp/registry.rs:172`），非缓存 list、连接测试/手动 warmup 时连接；stdio 在 `/Users/zmair/ZM database/kivio/src-tauri/src/mcp/conn.rs:319` 至 `:337` 启动配置 command/args/env。启动 server 的代码执行先于逐工具审批。
- legacy 设置 hooks 在下一次内置回复创建 dispatcher（`/Users/zmair/ZM database/kivio/src-tauri/src/chat/commands/reply.rs:752`），agent_start 等事件派发（`/Users/zmair/ZM database/kivio/src-tauri/src/chat/agent/loop_.rs:381`），`/Users/zmair/ZM database/kivio/src-tauri/src/chat/hooks.rs:451` 执行 shell，独立于工具审批策略。workflow hooks 则由其他插件/项目文件加载，不能混同“settings backup 导入所有 workflow hooks”。
- 最小建议：导入预览 enabled command MCP、shell/http hooks 和审批策略；未经逐项信任的新可执行来源导入为禁用。可信自备份恢复可显式保留原启用状态。不要只改工具审批，因为启动服务与生命周期脚本绕不开工具前的执行时机。

## 6. 主 WebView 未配 CSP：成立，P2 防御缺口；实际 XSS/RCE 未证明

- `/Users/zmair/ZM database/kivio/src-tauri/tauri.conf.json:13` app 没有 security/csp；`/Users/zmair/ZM database/kivio/index.html:4` 没有 CSP meta。
- 当前安装 tauri-utils 2.8.2 `src/config.rs:2579` SecurityConfig 派生 Default，`:2588` csp=Option，因此缺省 None。不是把未知默认当事实。
- `/Users/zmair/ZM database/kivio/src-tauri/src/provider_oauth/antigravity.rs:198` OAuth 回调响应有 default-src none，但不覆盖主 WebView。
- 最小建议：为主 WebView 建立实际资源清单后的 CSP（script/default/object/base/frame 等），兼容内联 bootstrap、图像、Mermaid 与 IPC，并做真实桌面回归。没 CSP 不等于已证实存在可注入脚本或突破 Tauri 权限。

## 验证范围

只读源码与已安装 Tauri 配置类型核验；没有 cargo、漏洞利用、真实备份导入、命令执行或密钥数据读取。生产代码没有改动。

## I/O 与进程生命周期只读复核（2026-10-10）

范围：读取当前任务 manifest/PRD/design/implement、工程规范及 ADR-0001/0002；未修改生产代码，未跑 Cargo。下列路径均以 `/Users/zmair/ZM database/kivio/` 为根。严重性是修复优先级，不意味着已观察到真实用户数据损失或远程攻击。

## 1. P1：Windows 原文件删除后提交失败会丢失两份内容

位置：`src-tauri/src/native_tools/files.rs:820` Windows 分支，`:826` remove_file(target)，`:829` rename(tmp,target)，失败时 `:832` remove_file(tmp)。调用：write_file `:357`、edit_file `:512` → atomic_write_text → atomic_write_bytes。

只要 unlink 成功后 rename 失败（例如目录权限/路径发生变化、临时文件被别的进程删除、应用在两步间退出），原文件已不存在；失败分支还会删除新内容临时文件。不是只有非原子的瞬间不可见，而是可能不可恢复丢失。Mac 构建不会执行该 cfg 分支，Windows 尚未实机复现。

注释“Windows rename 不能替换”不成立。[Rust 当前 1.99 官方 rename 文档](https://doc.rust-lang.org/std/fs/fn.rename.html)明确规定替换已有目标文件，Windows 对应 MoveFileExW 并回退 SetFileInformationByHandle。[Microsoft MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)的 MOVEFILE_REPLACE_EXISTING 支持替换已有普通文件。文档不能独立证明每种 Windows 文件系统、网络盘及断电场景的强原子/持久化保证，应分别验收。

最小修法：删除显式 target.exists/remove_file 分支，使用同目录临时文件直接 rename 覆盖，让提交失败保留旧目标；不是改成另一套 delete+rename。保留 sync_all，修正注释，Windows 验收正常覆盖、提交失败旧内容保留、占用目标/只读目标/并发变化。现有 `src-tauri/src/chat/storage.rs:172` 已采用直接 rename，并明确禁止先删除；二进制写入若统一实现，应在真实字节写入负责人收敛，不能为本修复扩展全仓重构。

## 2. P2：输出大小限制没有限制读取期内存

- 前台 bash：`src-tauri/src/native_tools/shell.rs:1076` child.wait_with_output；Tokio 1.49 源码 process/mod.rs:1449–1461 对两根管道 read_to_end 到 Vec。`:1114`/`:1115` 复制成 String，`:439` 完整格式化后才 offload_large_output。输出超过16KiB写日志只是限制返回给模型的文字，不限制读取期 RAM。未指定 timeout_ms 时可无限运行；例如普通大文件输出/持续输出可以持续占用内存直至停止或 OOM。没有实际制造 OOM。
- 后台 bash 已直接把 stdout/stderr 写日志（shell.rs:744），不是同一读取期问题；但 bash_output 在 `:872` fs::read 全日志，每次轮询即使 since_offset 已在尾部也先加载历史全部字节。长时间开发服务日志会让小增量检查付出全文件内存和 I/O。
- web_fetch：`src-tauri/src/native_tools/fetch.rs:181` 与 `:229` response.bytes() 先收全 body，之后 `:185`/`:233` 检查4MiB；错误响应 `:283` 同样无界 bytes 后才截为800字。最终120000字符上限 `:500` 也不限制下载期。30s超时限制持续时间，不能保证期间接收字节数。

最小修法：fetch 成功/reader/error 三路径共用有字节预算的逐chunk读取，Content-Length仅作早拒绝，仍对实际读取量计数；超限立即停止。前台 shell 同时持续排空两根管道，完整输出落盘并只保留有限tail/计数，保留现有脱敏、取消进程组、超时部分输出、退出码语义。后台读取使用 seek+有界增量，不再先读整个log；完整日志继续可按offset读取。

## 3. P2：HTTPS 验证只保护首跳；内网策略未定义

`src-tauri/src/native_tools/fetch.rs:153` 只检查初始URL scheme与host存在，`:165`用传入共享Client发送。`src-tauri/src/api.rs:201` builder未配置redirect或https_only。当前reqwest0.13.4源码 redirect.rs:160 默认最多10跳，async_impl/client.rs:372 https_only=false；HTTP/HTTPS下一跳被接受。因此公共 HTTPS URL 可302至 HTTP localhost，绕过该工具“only allows https”承诺。直接的 https://127.0.0.1、私网IP、域名解析到私网也未过滤（TLS仍照常验证，不意味着任意自签HTTPS可连接）。

内网可访问是事实，但桌面助手可能需要访问用户内网；未找到公共互联网专用边界，不能未经产品决定宣称所有内网访问都是违规。共享Client不携带浏览器cookie，不应夸大为浏览器身份自动泄漏。外部网页控制重定向至未认证本地HTTP服务并将响应作为工具结果返回模型，构成需明确处理的跨网络边界风险。

最小修法：在 fetch 自有传输配置中禁止自动重定向或逐跳检查 HTTPS；不能全局禁止共享Client HTTP，否则会破坏本地模型/MCP。若决定仅允许公网，还须在首跳及每次重定向拒绝特殊/内网地址，并对实际DNS连接目标约束，单独预查询DNS后交给另一轮解析不能防止重绑定。reader fallback默认开启（parse_args:149），访问私有URL时会把URL发给 r.jina.ai；若保留私网支持，应禁止其自动外部reader回退，避免内部URL/查询参数外传。

## 4. P2：部分 macOS 进程关闭只杀直接子进程

MCP：`src-tauri/src/mcp/conn.rs:332` kill_on_drop=true，但普通Command未设置独立session/group，也未包装process-wrap ProcessGroup。rmcp3.5.1 child_process.rs:150 使用传入CommandWrap，process-wrap10.0.1 generic_wrap.rs:320 From<Command> 创建空wrappers；SDK并不会自动树杀。rmcp关闭等待/kill针对直接Child。`mcp/manager.rs:968`取消service→transport.close不能证明其孙进程退出。若npx/sh/自建server启动子进程且未自行清理，会留下孙进程。正常协议tools/call取消保留MCP server是合理行为，不应每次Stop都杀整个常驻server。

外部CLI范围（当前注册10种）:

| 路径/CLI | 创建与退出证据 | 判断 |
|---|---|---|
| Claude persistent | session/claude_stream.rs:711普通cli_command；:1408超时kill_agent_process_tree | 无独立组；正常EOF退出可能CLI自行清理，但超时fallback不能保证孙进程 |
| Codex app-server | session/codex_app_server.rs:1499普通command；:1967超时tree helper | 同上 |
| Pi RPC | session/pi_rpc.rs:1858普通command；:1935 tree helper | 同上 |
| DSH JSON-RPC | session/dsh_jsonrpc.rs:261普通command；:945 tree helper | 同上；dsh_profile.rs:636的process_group仅用于安装，不保护该会话 |
| ACP: opencode/gemini/kimi/hermes/grok | registry.rs列出对应defs；session/acp.rs:1483普通command；:1882 start_kill | 只杀直接CLI；ACP独立terminal子命令是另一个有setsid的受保护路径 |
| Antigravity session | session/antigravity.rs:280 process_group(0)，:493 tree helper | 已建独立进程组，不归入缺失组问题；主动setsid脱离组的后代仍不受组信号约束 |
| 通用一次性spawn_agent及probe | external_agents/spawn.rs:564/:579未建组 | 同类风险，具体是否有后代取决于CLI实际行为 |
| 原生bash、ACP terminal | native_tools/shell.rs:1064及:761 setsid；session/acp_terminal.rs:419 setsid | 已建session，有整组kill；不能泛称全项目取消泄漏 |

根因：external_agents/spawn.rs:217 helper调用kill_process_group(pid)，shell.rs:702/703实际为kill(-pid,signal)。要求PGID恰为该PID，而普通子进程继承父组；helper注释:211自己承认spawn_agent没有setsid却仍宣称“整棵树”。fallback start_kill只杀直接Child。live.rs:480退出超时只调用组kill，无组会连直接子进程都无法立即强杀。

本机真实实验（Darwin 27.0.0，非Linux）：受控Python父进程普通spawn一个持有孙进程的子进程，观察direct PID65420/PGID65418，grandchild PID65421/PGID65418；killpg(65420,SIGTERM)返回ESRCH，直接kill/reap65420后65421仍存活。随后显式SIGKILL孙进程；再次kill(pid,0)确认65420、65421都不存在。该实验验证Unix语义反例，未运行实际Claude/MCP工具，不代表每个CLI正常退出必然泄漏。

最小修法：真实进程生命周期负责人创建独立Unix组/session，并在强杀、握手失败、异常Drop与应用退出使用同一组/句柄关闭契约，保证leader仍存活时先终止组再reap；MCP可使用SDK接受的ProcessGroup/ProcessSession CommandWrap，不能只加process_group(0)却继续SDK直接Child.kill。Windows另验JobObject/进程树，不能从mac实验推断。保持正常轮次协议取消、原生历史落盘、跨轮常驻与ADR所有权语义。验收需fake CLI/MCP生成孙进程，覆盖超时/关闭/握手失败，并确认受控后代终止；任意daemon主动脱离session的情形单独声明。

## 协议、历史与存储复核

No production edits or Cargo commands in this review. Read current task manifests/PRD/design/implementation notes and traced live serializers/storage callers. This review does not claim a reproduced official-provider HTTP 400; no official Anthropic credential was available. Previous signed-thinking regression results remain the evidence for the completed fix, rather than being rerun here.

## Findings

### Missing tool results: incomplete histories still cross OpenAI/Gemini boundaries

- Shared conversion: `/Users/zmair/ZM database/kivio/src-tauri/src/chat/model/types.rs:837` invokes `repair_legacy_tool_result_order`; its implementation at `:884` only moves existing matching results before the next assistant. The comment at `:879` explicitly excludes inventing missing results.
- OpenAI actual request body: `/Users/zmair/ZM database/kivio/src-tauri/src/chat/model/openai.rs:557` calls `openai_messages_from_generate_request`, defined in `model/types.rs:929`. It serializes existing messages and strips foreign metadata; it neither validates nor supplies absent results. Responses likewise directly serializes `function_call`/`function_call_output` from canonical parts (`model/types.rs:1339`).
- Gemini actual body: `/Users/zmair/ZM database/kivio/src-tauri/src/chat/model/gemini.rs:396` → `contents_with_tool_ids` at `:682` → `gemini_parts` at `:765`/`:792`. It merges adjacent roles but never validates result completeness. Its ID map at `:701` helps name lookup only. Native Gemini currently omits function-call IDs; the include-ID branch is specifically the Antigravity Claude/gpt-oss path.
- Anthropic alone synthesizes/reorders missing results at `/Users/zmair/ZM database/kivio/src-tauri/src/chat/model/anthropic.rs:1104` → `ensure_tool_result_pairing` at `:1113`.

Judgment: confirmed serializer gap for already incomplete/legacy/interrupted history. Complete normal tool batches do not demonstrate this defect. Strict endpoints may reject the next request rather than recover. OpenAI's official guide demonstrates one tool response per returned call; Gemini's schema requires responses to correspond to predicted calls. These sources support the pairing contract, but this audit did not produce a live 400 or prove every gateway rejects the same malformed history. Prefer validating/repairing at the common typed history seam instead of three divergent endpoint policies.

Sources: [OpenAI function calling](https://developers.openai.com/api/docs/guides/function-calling), [Gemini FunctionCall/FunctionResponse schema](https://ai.google.dev/api/generate-content).

### Foreign tool IDs: Anthropic client ID constraint confirmed; no live 400 claim

`functions.x:0` is copied verbatim into `tool_use.id` by `/Users/zmair/ZM database/kivio/src-tauri/src/chat/model/anthropic.rs:1804` and into `tool_result.tool_use_id` by `:1818`. No normalization/mapping was found.

The main session found the expanded official [Messages CLI schema](https://platform.claude.com/docs/zh-CN/api/cli/messages/create): under the client `tool_use_block` its `id` has pattern `^[a-zA-Z0-9_-]+$` (retrieved lines 673–681), independently of the server-tool `srvtoolu_` pattern. Dot and colon violate this client constraint. This is a confirmed provider-boundary compatibility gap when switching transcripts containing such IDs to Anthropic, although this audit did not send a live request to reproduce HTTP 400. Map call and result IDs consistently, preserve uniqueness, and do not mutate signed native content arbitrarily.

### Empty assistant/native-filtered message becomes an empty Anthropic text block

`/Users/zmair/ZM database/kivio/src-tauri/src/chat/model/anthropic.rs:1858` unconditionally supplies `{type:"text",text:""}` when conversion produces no blocks. This is reachable from an empty assistant, generic unsigned reasoning excluded on Claude, or a native-reasoning-only assistant whose provenance no longer matches after a model/provider switch. `:1104` pairing repair does not remove empty text. The caching helper correctly avoids cache control on empty text; that does not remove the block itself.

Judgment: confirmed malformed/empty payload construction; likely official Anthropic non-empty-text validation failure, not newly reproduced here. An [issue in Anthropic's own Claude Code repository](https://github.com/anthropics/claude-code/issues/50010) records the endpoint rejecting an empty text block with 400, but it is a reported incident rather than this repository's live integration proof. Gemini drops empty part messages at `gemini.rs:692`; this precise empty-block defect is Anthropic-specific.

### Signed-thinking fix is intact within its supported contract; server-tool continuation remains outside it

No residual signature/order defect was found in the previously fixed thinking/redacted/client-tool contract. New review boundary: `/Users/zmair/ZM database/kivio/src-tauri/src/chat/model/anthropic.rs:1260` only retains `thinking`, `redacted_thinking`, plain `text`, and client `tool_use`; `:1306` drops other native blocks. Text parsing at `:1288` drops native citations as replay metadata. Therefore native `server_tool_use` and `web_search_tool_result` are not losslessly persisted/replayed, even though built-in search can retain display citations separately. Streaming assembly ultimately uses this same decoder. `finish_reason_from_anthropic_stop_reason` at `:2025` maps `pause_turn` to the default `stop`.

Impact: a server-tool `pause_turn` response may be treated as completed instead of continuing the original native content. This is a separate capability/protocol gap, not proof that ordinary extended-thinking client-tool rounds remain broken. Whole runtime history also remains `Vec<Value>` by the explicitly approved previous scope.

Sources: [Messages native content blocks and thinking requirements](https://platform.claude.com/docs/en/api/messages/create), [server tools continuation](https://platform.claude.com/docs/en/agents-and-tools/tool-use/server-tools).

### Draft journal reset is non-atomic; main conversation revision already exists

- Real partial persistence: `/Users/zmair/ZM database/kivio/src-tauri/src/chat/commands/messages.rs:537` invokes journal append from an awaited blocking task.
- `/Users/zmair/ZM database/kivio/src-tauri/src/chat/draft_journal.rs:52` checks the 16 MiB cap; `:56` resets via `fs::write`, which truncates the old journal before completing the new snapshot. A crash/write failure during reset can destroy the old recoverable snapshot; partial trailing-line tolerance cannot recover a completely truncated journal. This is a static atomicity defect, not an injected crash result.
- Journal JSON is just a `ChatMessage` (`:49`), without base revision/run generation. Recovery `:107` skips an ID present in the main file; `:112` upserts an absent ID without expected revision; `:122` deletes the journal afterward. A stale journal can thus be treated as eligible if its message was deliberately removed from main history. Startup recovery is spawned asynchronously at `/Users/zmair/ZM database/kivio/src-tauri/src/lib.rs:396`, rather than awaited as a setup barrier; the comment saying setup has no active run is not a synchronization primitive. The precise overlapping UI/run scenario was not reproduced.
- Main conversation does have revision and serialization: `/Users/zmair/ZM database/kivio/src-tauri/src/chat/types.rs:616` defines revision with legacy default; `/Users/zmair/ZM database/kivio/src-tauri/src/chat/repository.rs:789` acquires the conversation lock, `:804` checks optional expected revision, `:815` increments and `:817` persists. Final assistant persistence at `commands/messages.rs:377` uses repository mutation; `:397` removes the journal after main write. Do not report the entire store as unversioned/non-atomic.

Prioritize temp-file atomic replacement for journal reset. A versioned journal envelope and recovery barrier are separate hardening work; do not infer all existing conversation operations lack CAS.

### Token estimation/headroom: conditional undercount, not universally late compaction

`/Users/zmair/ZM database/kivio/src-tauri/src/chat/agent/prepare.rs:877` estimates ceil(ASCII/4) + one token per non-ASCII character. This is a heuristic, not a tokenizer upper bound. Image parts deliberately contribute zero in this estimator (`:890`); UI measurement adds separate image costs, but that does not make the runtime text estimate include them. Signature/opaque bytes are appropriately excluded rather than counted as base64 input.

`/Users/zmair/ZM database/kivio/src-tauri/src/chat/agent/compaction.rs:26` reserves max(15%,16384) with a small-window fallback to 15%; e.g. 8K budget=6800, not an unusable 16K reserve. `:829` adds tool schema cost; `:831`/`:840` use provider/persisted usage anchors, and `:851` takes the larger anchor/local estimate. `:852` tests before requests (planning caller `planning.rs:74`); overflow also has reactive forced compaction. Therefore a blanket claim that compression is only late or ignores provider usage is false.

Residual: reserve is independent of actual `config.max_output_tokens` (sent at `/Users/zmair/ZM database/kivio/src-tauri/src/chat/agent/planning.rs:170`; GenerateOptions writes it at `:524`). On a small-window model requesting output larger than the 15% headroom, or a fresh history with many images/poorly tokenized text and no valid usage anchor, a sub-threshold estimate can still produce an oversized first request. Impact is initial rejection/reactive retry and possibly a turn that cannot compact enough, rather than deterministic failure on every small window. No real-tokenizer benchmark or provider overflow reproduction was run in this read-only audit.

## Recommended ordering

1. Empty Anthropic block avoidance and common missing-result validation/repair: direct next-request failure boundaries.
2. Atomic journal reset: recoverability regression under crash/write failure.
3. Output-aware small-window headroom and initial image budget; source-specific server-tool continuation if that capability is intended.
4. Foreign client ID mapping (schema now verified), and versioned journal/recovery barrier; keep live failure claims separate from static verification.
