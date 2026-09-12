# SubAgent 实施与验收记录

日期：2026-09-12。依据本地定稿规格及 01—12 任务，按用户后续 `implement` 指令实施。基线：`118d4874b82371a9a57b9879450dc892ca3fb73d`。未发布外部任务、未推送代码、未部署。

## 已实现的行为

- 内置 `agent` 只负责受理，立即返回子代理身份；执行交由进程持有的 supervisor，复用共享 Agent loop。新增 `agent_control` 支持 list/get/message/continue/stop/wait。
- 子代理身份、执行、历史、消息、工具意图/结果与终态分别可追踪。完整记录原子写入文件，轻量索引可重建；详情按需读取。
- 并发默认 12、范围 1—64；容量满立即拒绝，空闲身份不占执行名额。重试派工及消息按稳定键去重。
- 主轮在安全边界收取已结束结果，等待必要依赖时可响应新输入；活跃主轮的结果保留给原所属主轮。父对话中的结果消息同时充当持久接收凭据，写入成功后才确认子方 outbox。
- 独立停止和主协作停止保留资源归属直到工具/worker 退出；存储故障不能阻断内存取消。终态保存故障仅重试保存，不重跑工具或模型。
- 完成后保留身份和历史；普通留言不启动模型，明确继续才产生新执行。重启将未终态执行转为中断，不自动恢复调用。
- 工具执行前记录结果未知，返回后保存结果；checkpoint 记录结果已进入上下文，防止继续时复原被压缩掉的旧内容。
- 当前对话面板提供状态、详情、消息、停止、继续、重新连接及中英文文案。关闭面板不停止执行；旧历史卡片和外部 CLI 路径保留。

## 验证范围

确定性测试使用生产 Runtime、实际文件存储、生产 supervisor、共享 Agent loop，以及本地 HTTP provider / 受控工具边界。React 测试通过用户点击及输入检查 API 操作。未将这些测试称作真实远程 provider 或原生桌面端到端验证。

实际远程 provider 调用、真实桌面退出/重开操作、所有外部工具进程的强制取消均未在本轮实测。对于不能确认已终止的工具，界面保持“正在停止”，执行名额不提前释放；退出/删除若五秒内未完成清理会明确失败，可稍后重试。

## 检查结果

- `npm test`：199 个文件、1531 项测试通过；随后新增重连测试后，面板与旧工具卡片最终回归 43 项通过。
- `npx eslint src/chat/SubAgentPanel.tsx src/chat/SubAgentPanel.test.tsx --max-warnings 0`：通过。
- `npm run typecheck`：通过（协议导出一致性 + TypeScript）。Rust 最终全量：2356 通过、5 失败、44 忽略；其中 48 项 SubAgent 相关测试全部通过。OAuth 端口占用失败随后单独复测通过，剩余 4 项见下。
- HTTP hooks 的 502 故障与本机代理有关；仅在测试进程设置 `NO_PROXY=127.0.0.1,localhost` 后，3 项 HTTP 测试通过，未改变系统代理设置。
- `git diff --check`：通过。

最终全量的 OAuth 回调测试遇到 `AddrInUse`，单独复测通过。仍未解决的四项为：

- `chat::model_metadata::tests::model_database_matching_is_isomorphic_with_frontend`：模型别名期望不一致。
- `chat::vision::tests::auto_auxiliary_vision_picks_enabled_vision_model_when_main_is_text_only`：视觉模型选择期望不一致。
- `path_env::tests::common_dirs_macos_expands_home`：Windows 环境中的 macOS HOME 展开期望。
- `path_env::tests::merge_unix_from_minimal_path_adds_common_dirs`：Windows 环境中的 Unix 路径拼接期望。

对应源码相对实施基线无改动；本轮没有修复这些无关测试，也未进行独立基线构建，不能宣称全量测试全绿。

原始日志保存在本目录的 `*.log`，不纳入提交。最终日志为 `rust-complete.log`、`frontend-full.log`、`ui-final.log`、`typecheck-final.log`、`panel-lint.log`；环境复测为 `hooks-local.log`、`oauth-isolated.log`。

## A01—A30 证据索引

“确定性”表示该接缝已经有可执行回归测试；“代码审查”表示接入路径已经静态核查，并不等于原生桌面实测。以下不批量宣称所有场景均已端到端验收。

| 编号 | 实现与可核查证据 | 验证边界 |
|---|---|---|
| A01 | `real_supervisor_returns_before_work_finishes_and_notifies_independently`；`control::launch` | 生产 supervisor + 屏障；主工具入口代码审查 |
| A02 | 同一 supervisor 测试分别释放 A/B；`collect_results` | 独立完成确定性；主方接入代码审查 |
| A03 | `accepted_start_is_idempotent_and_survives_restart`；实际 supervisor 重复启动测试 | 真实存储 + 执行次数 |
| A04 | `Runtime::resume` 在 admission 锁内判定 active，`claim_worker` 唯一执行 | 代码审查；继续/旧停止确定性测试 |
| A05 | `messages_are_consumed_once_and_late_messages_remain_idle`；loop checkpoint | 消费和历史原子保存确定性 |
| A06 | 同上；面板 `idle information is a message; explicit continuation starts work` | 持久化与 UI 操作确定性 |
| A07 | `pending_message_at_final_boundary_continues_current_execution`、`failed_worker_keeps_late_input_pending_for_explicit_continuation` | 最终边界与晚到留言确定性 |
| A08 | `accepted_message_survives_secondary_summary_write_failure`、`failed_primary_write_does_not_acknowledge_input` | 真实文件写入故障与重开 |
| A09 | `event_between_snapshot_and_wait_is_not_lost`；先订阅后快照 | watch 确定性 |
| A10 | `control::operate(wait)` 用截止时间/新输入结束等待，不执行 stop | 代码审查 |
| A11 | `stopping_one_child_keeps_others_running_and_holds_capacity_until_finished` | 多子任务隔离确定性 |
| A12 | `user_stop_requires_user_continuation_and_old_stop_cannot_touch_new_run` | 预期执行校验确定性 |
| A13 | `stopping_parent_seals_new_admission_without_affecting_other_conversation`、`parent_stop_cancels_every_worker_even_when_storage_is_offline` | 容量与存储故障确定性；launch 取消竞争代码审查 |
| A14 | 用户停止标记持久化；主代理不能自动继续，用户面板或主代理显式声明新的用户继续指示可解除，并记录来源 | Runtime 确定性；`main_agent_can_relay_a_new_user_continuation_without_impersonation` |
| A15 | `managed_worker_stop_keeps_tool_owned_until_it_returns`；shutdown/delete 等待 supervisor | 真实 loop + 受控工具；桌面退出未实测 |
| A16 | ChatAgentHost checkpoint 等待当前 parent_run 的 active 依赖，终态包含 failed/interrupted | 代码审查；实际多模型主轮未远程实测 |
| A17 | supervisor 独立写终态；没有结束子任务自动启动父模型的路径 | 真实 Runtime 确定性 + 代码审查 |
| A18 | `terminal_replay_is_idempotent_and_cannot_regress_status`、`parent_receipt_and_child_outbox_recover_both_sides_of_a_crash`、`live_parent_results_are_not_claimed_by_other_model_arms` | 真实文件接收事务与重开；活跃主轮归属 |
| A19 | 面板 unmount 不 stop；重连读取 list/get，输入框标记与任务页共享快照轮询 | React 操作测试；原生窗口未实测 |
| A20 | `restart_retains_pending_messages_and_unknown_tools_without_replaying` | 实际文件重开，无自动调用 |
| A21 | `subagent_empty_planning_recovery_does_not_repeat_tool_work` | 真实 loop、本地 HTTP、工具执行次数 |
| A22 | 工具调用前 durable intent；重开保留 unknown 并明确核查提示 | 真文件恢复确定性；不承诺外部副作用恰好一次 |
| A23 | capacity、panic、stop、真实 supervisor 测试 | 原子预留及清理后释放确定性 |
| A24 | `capacity_is_admission_not_a_hidden_queue`；去除旧 agent 专属 660 秒等待 | 确定性 + 代码审查 |
| A25 | `get/send/resume/stop/checkpoint` 统一 scoped 校验；旧执行停止校验 | 跨对话与过期目标确定性 |
| A26 | `continuation_preserves_compaction_and_does_not_resurrect_old_tools`；详情保存全文，模型输出有界 | 真文件/压缩历史确定性；用量按执行保存 |
| A27 | `prepare_continuation` 验证当前 provider/凭据，工具与当前允许目录取交集 | 代码审查；真实凭据撤销未实测 |
| A28 | Tauri 命令与 native tool 都调用 `operate`；Panel 调用类型化 API | 代码审查 + UI 操作测试 |
| A29 | `subagent_started` 回执渲染，旧 ToolCallBlock 测试 40 项；外部 CLI 未替换 | 前端回归 + 代码审查 |
| A30 | `terminal_storage_retry_keeps_result_and_never_reexecutes_work`、父接收事务故障测试、索引故障恢复测试 | 真实文件故障；终态存储成功前保留资源归属 |

## 双轴审查

### Standards

初审指出模型输出上限、共享 UI 组件、中央 API 类型三个规范问题，均已修复。结果收集逻辑移至子代理控制模块，避免 ChatAgentHost 承担存储协议。前端 lint 检查通过。最终暂存版本经独立规范复核，以上三项均确认关闭，限定复核范围无残留发现。

### Spec

初审及复审指出持久接收凭据、退出/删除清理、已返回工具恢复窗口、全历史 eager loading、活跃主轮结果归属、压缩后旧工具重建、受理后取消丢失 supervisor、存储故障漏停后续任务；均已修复并新增对应回归测试或静态复核。最后一轮静态复核没有新增阻塞发现。补充复核确认失败边界保留待消费留言，以及主代理明确转达用户继续指示的来源标记；`user_requested` 是模型依据用户原文作出的声明，并非后端独立语义验证。

## 本地交付

2026-09-13 用户试用后的 UI / 等待修复与验证见 [后续修复记录](followup-ui-wait.md)。

任务 01—11 的实现已接通，12 的验证证据在本文集中记录。保留原任务清单以便查看原始要求，不用统一勾选掩盖未实测环境。远程 provider / 原生桌面的未验证范围保留为交付限制，不伪称已验收通过。
