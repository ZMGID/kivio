# 外部 CLI 维护变更整体审查

用户目标是完成维护计划剩余任务，参考 T3code 优化现有实现，验证并提交，再整体审查这部分变更。第 6 项已在 `168ae23b1761e481d06c89376628e074babd84c5` 提交；本次整体审查覆盖基线 `7a7e10b81b37b35758dff840a32d2e07138bdee6` 到该提交的第 1 至 6 项实现。

审查按协议与生命周期、安装与运行身份、历史与交互三组独立检查，另对修复做独立复核。检查范围包括普通发送、后台任务、自动化和语音调用方，设置与安装前端、账号查询、原生配置事务、导入、分支、回退及迟到回执。参考版本、各任务验证及平台限制见[任务记录](../prd/external-cli-maintenance-plan.md)，回放样本的版本和真实性分级见[样本说明](../../src-tauri/src/external_agents/fixtures/README.md)。

## 确认的问题及修复

| 级别 | 触发与影响 | 原入口中的修复 |
| --- | --- | --- |
| P2 | Windows 手填 `codex_home` 等大小写别名指向自定义目录时，凭据指纹可能仍读取默认目录；供应商变量与手填别名也可能共存，实际启动值不确定。 | `overrides` 按 Windows 规则合并手填覆盖并保留变量拼写，启动身份使用标准库 Command 的平台环境语义；`provider_profile` 按平台规则查找 home。歧义校验仍检查原始设置，不因合并丢失冲突；Codex home 检测及 WSL 路径转换同步识别别名。 |
| P2 | dsh 使用自定义 `DSH_HOME` 且缺少插件时，插件安装仍写入继承的默认目录，随后读取自定义目录失败。 | `ensure_profile_ready` 解析一次 home，传给现有安装命令；继续移除供应商凭据，安装与读取使用同一目录。 |
| P2 | 分支或回退的待补历史以某条回答为边界，删除该回答后边界失效，下一次发送失败；修改待补前缀也可能让旧回执结清新历史。 | 删除入口更新边界和 transition ID，改为文本补历史并保留不确定投递状态。发送准备复用现有恢复事务修复已落盘的失效边界，旧回执不能结清修改后的状态。 |

四个新增回归用例在修复前全部失败，分别证明自定义凭据变化未被识别、手填别名未覆盖、安装命令缺少目标 home、删除边界导致补历史失败。随后补充旧回执隔离用例，并在原有真实文件恢复测试中覆盖已保存的失效边界。用例使用临时配置、真实文件及生产命令配置和历史入口，不读取或改写用户账号，不调用真实包管理器。

修复过程中，全套测试发现环境变量过早归一化掩盖已有歧义检查；已改为先检查原始设置，运行时再合并，不降低原有保护。

## 验证与结论

最终 Windows Rust 全量 lib 测试 **3221 项通过，56 项按既有配置跳过**。前端聊天、设置与 API 的 **2648 项测试通过**；后续修复没有修改前端。架构检查及差异空白检查通过。独立修复复核未确认新的 P 级缺陷。

第 6 项提交前已通过协议一致性、TypeScript、相关文件 ESLint 和 Clippy；修复后的协议一致性及 Clippy 复核也通过。Clippy 允许的存量及风格警告不作为零警告声明。首轮修复全套运行另有一项未修改的 shell 进程清理测试失败，最终完整重跑该项及其余测试均通过；没有为此放宽断言或删除测试。

本地验证与独立审查证据保存于 `.scratch/external-cli-step6/`：`review-regressions-before.log` 为四个失败复现，`review-fixes-full2.log` 为最终完整 Rust 结果，`frontend.log` 为前端结果，`review-protocol.md`、`review-install.md`、`review-history.md` 为三个整体审查分区，`review-fixes.md` 为独立修复复核。该目录是忽略的本机工作记录，本报告保留可随提交阅读的结论；复跑使用项目现有测试命令及样本说明。

协议与生命周期组未确认其他新增缺陷。审查按统一工程规范检查归属：诊断归现有管道资源，回放替换外部 I/O；安装仍由 installer 和配置事务负责，历史仍由 Conversation 与仓库提交负责。没有新增运行框架、依赖或平行配置机制。

Laziness Protocol 促使本次复用现有管道、actor、安装命令及恢复事务；Model the Domain 将诊断观察窗口与原生请求归属区分，并保留历史投递的不确定状态；Fix Root Causes 将修复落在环境合并、安装目标和历史修改入口；Prove It Works 要求先复现四个失败，再通过实际生产入口回归。

## 后续实机回归：OpenCode 发送栈溢出

用户在 Windows Dev 选择 OpenCode 发送消息后报告闪退。通过既有 `chat_probe` 调用同一聊天生成入口，发送不读写文件的最小消息，在未修改的代码上复现 `STATUS_STACK_OVERFLOW`。此前的协议和历史单测没有覆盖这个完整发送调用栈，自动化通过不能替代实机发送验收。

完整外部运行 future 被 task-local、取消等待和上层异步调用内联保存，调试构建的栈帧叠加超出默认线程栈。临时大栈与插桩用于确认有限栈压力；仅装箱下层连接任务不足以解决问题，已撤回。最终只在 `run_external_cli_reply_in` 中、`PROCESS_CONVERSATION.scope` 之前用 `Box::pin` 保存完整 owned future。取消、落盘收尾、lease 和上下文传播保持原路径，没有新增任务或修改全局栈大小。所有插桩均已删除；实际验证的最终 exe 主栈恢复为原来的 1 MiB，启动环境没有 `RUST_MIN_STACK` 覆盖。

默认栈下，本机 OpenCode 2.2.5 的首轮返回 `OK`，常驻会话存活；已有 `probe:e2e` 的 `multi-turn` 场景通过，同一进程续聊并记住上文。取消轮返回 `cancelled`，随后在同一对话恢复原生会话并正确返回先前记住的 `77`。已有 `cancel-resume` 场景按 Claude 语义要求取消后保留同一 PID，因此该断言在 OpenCode 上报红；ACP 的现有 `cancel_keeps_live_session` 策略本就关闭旧进程并在下轮恢复。本次没有修改或放宽该脚本，另通过同一入口验证了 ACP 的正确恢复行为。

修复后的 Windows Rust 全量 lib 测试 **3221 项通过、56 项按既有配置跳过**，架构检查和差异空白检查通过。新的独立审查未发现取消、Drop、lease、task-local 或错误传播回归。动态证据为本机 `.scratch/dev-20261107-043504.stderr.log`、`opencode-stack-final-result.log`、`opencode-stack-live-regression.log`、`opencode-stack-after-cancel-result.log` 和 `opencode-stack-rust-final.log`。另一个工作区一度占用相同 Dev 身份，误由该实例消费的 `opencode-stack-outer` 请求已排除；最终验证按当前 checkout 的可执行路径和 PID 核对归属。

## 第二轮审查修复

第二轮审查以 `fdd8ce5b` 为基线，再次核对计划六项内容。确认的四项 P2 已建立失败回归，先提交测试，再在现有负责人内修复。

| 问题 | 修复归属与行为 |
| --- | --- |
| ACP 首次补历史在配置请求断线后丢失。原生空会话恢复成功，被误当作历史已经交付。 | `run.rs` 在明确未发送的恢复重试中保留本次完整提示词。新建原生会话仍使用首轮提示词，可能已发送的轮次仍禁止自动重试。成功回答落盘后才结清匹配历史。 |
| OpenCode、Grok、Kimi 的自定义 HOME 文件变化未进入启动身份。 | `provider_profile.rs` 让配置物化与启动指纹共用路径规则，指纹读取已捕获环境中的实际路径。继续保留专用目录优先级与 Windows 变量大小写规则。 |
| 失败或取消轮次保存部分回答后返回成功，自动化继续下游，手动压缩更新成功时间。 | 共同运行入口返回明确的已持久化终态。自动化与压缩要求完成终态，普通聊天继续消费已经保存的回答与错误记录。 |
| 诊断把 `conv_<uuid>` 当作裸 UUID，省略宿主对话归属。 | `diagnostics.rs` 验证并保留完整宿主 ID，包括自动化 ID。非法或超过 256 字节的 ID 省略，原有脱敏和容量上限不变。 |

参考仍使用计划中的固定 T3 快照。历史交付参考 [ContextHandoffDelivery](https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/apps/server/src/orchestration-v2/ContextHandoffDelivery.ts)，终态参考 [ProviderAdapter](https://github.com/pingdotgg/t3code/blob/6577c6b6185167c0e502fa01f9e8e8f2dee6ff25/packages/provider-core/src/server/ProviderAdapter.ts)，有效目录参考同版本的 ClaudeHome 与 CodexHomeLayout，诊断归属参考 [ProviderEventLoggers](https://github.com/pingdotgg/t3code/blob/85bd3f2a2a0ad4a8ddf2950e01db80765e9a1bf4/packages/provider-core/src/server/ProviderEventLoggers.ts)。复用的是交付、终态和身份约束，没有引入 T3 的服务框架或新依赖。

新增运行回归在 Windows 测试进程内启动临时 Python ACP 对端，通过真实管道、生产 actor、共同运行入口与临时对话仓库验证。对端只返回固定数据，不执行真实 CLI 或模型请求。HOME 回归使用临时配置文件与真实启动身份计算。修复前四个 HOME 测试失败，五个运行测试中四个命中预期失败，部分回答保存的对照通过。

修复后，11 项针对性回归全部通过，Windows Rust 全量 lib 测试 **3232 项通过、56 项按原配置跳过**。成功、失败和协议取消均检查持久化回答与单次终态。压缩失败保留原时间、警告、计数和边界；外部上下文原有的 `summary = None` 重算规则不变。HOME 用例另覆盖专用目录优先级、大小写别名和捕获路径之外的文件变化。诊断测试覆盖普通与自动化 ID、无效输入及长度边界。

独立正确性复审未发现新的 P1/P2。注释复审移除一条重复叙述，保留公共返回契约与日志长度限制说明。另修正上一轮指出的历史回归用例格式遗漏，没有格式化无关代码。Laziness Protocol 促使路径计算继续归原模块，Type System Discipline 将持久化收尾与执行成功分开，Fix Root Causes 将修复落在共同运行入口，Prove It Works 通过相同生产调用路径验证先失败后通过。

协议一致性、TypeScript、架构、Clippy 全目标检查及差异空白检查通过。Clippy 仍报告非阻断警告，不作零警告声明。新增回归文件、诊断、profile 与历史生命周期测试文件的格式检查通过；全仓 `cargo fmt --all -- --check` 仍因多处存量格式差异失败，未将其报告为通过，也未扩大本次修改范围。

本轮证据保存在忽略目录 `.scratch/external-cli-review-fixes/`。`home-red.log` 与 `runtime-red-2.log` 是有效失败复现；较早的 `runtime-red.log` 是测试夹具初始化失败，不计入缺陷证据。`regressions-green.log` 与 `full-rust.log` 保存修复后结果，`final-correctness-review.md` 保存独立复审结论。测试使用普通 Windows libtest 入口，ACP 夹具要求 PATH 可找到 Python 3。

本轮没有启动或操作 SDK 工作树的 Dev，没有执行真实 CLI 账号、安装或模型请求。新增取消用例由协议对端报告取消，不单独证明用户点击停止的时序。Pi 连续压缩拒绝、Codex skill slash 等待窗口的后台审批，以及旧安装旁路占用仍属于前轮列出的待验证风险，本次没有将其宣称为已修复。

## 验证边界

除前述 OpenCode 的实际模型请求外，未运行其他 CLI 的真实模型请求、登录、安装更新或桌面完整联调，未验证 WSL、Linux/macOS 实机及远端 CI。本机桌面控制接口不可用，OpenCode 补验使用已有调试入口进入相同后端生成路径，未自动操作聊天窗口。Windows 受控子进程测试不能证明 WSL 内后代进程清理；路径转换测试也不等于 WSL 实机验证。各 CLI 样本仅证明文档列明的版本及片段，不证明任意升级版本兼容。诊断摘要有损且容量有限，不是完整会话录制。

独立审查使用当前可用的 GPT 家族模型，未执行跨模型家族评审。没有发现其他确认问题不等于证明全部路径无缺陷。既有平台限制、未托管终端的并发修改、宿主崩溃时跨文件原子性及 npm 包版本回滚仍按任务记录中的边界处理。
