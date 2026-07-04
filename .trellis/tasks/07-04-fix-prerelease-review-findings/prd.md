# PRD — 修复 v2.7.4→HEAD 审查发现的正确性问题

## 背景
对未发布改动（`v2.7.4..HEAD`，约 16.6k 行 / 131 文件）做了分主题代码审查。
本任务只处理**已逐条对照源码核实为真**的正确性/可用性问题；纯观感项与需单独设计
的项列入 Deferred，不在本次范围。

## 范围内（confirmed real，须修）

- **F1（高）`src-tauri/src/chat/model/openai.rs:429`** —
  只要有 `conversation_id` 就无条件同时下发 `prompt_cache_key`（OpenAI 真参数）与
  `promptCacheKey`（驼峰，**非**真实参数）。真实 OpenAI / Azure / 校验型代理对未知
  body 字段返回 400，可致整条聊天不可用。这正是当初逼出原生 gemini 适配器的同一类问题。
  - 修法：去掉驼峰 `promptCacheKey`（仅保留标准 `prompt_cache_key`）。

- **F2（中）`src-tauri/src/chat/commands.rs:4497` `group_answer_excluded_from_context`** —
  多答组无显式 `group_selections` 时，回退取"组内顺序第一条 assistant"，未跳过
  `stream_outcome == "error"`。首臂报错时，错误文案会作为上一轮答案回灌给模型。
  - 修法：回退查找时跳过 `stream_outcome == Some("error")` 的条目；若全组皆 error 才退化到第一条。

- **F3（中）`src/onboarding/OnboardingShell.tsx` `persistSettings`/`handleFinish`/`handleSkip`** —
  `save_settings` 在热键冲突时会 `Err` 并回滚，但前端 catch 后仅返回 false、无任何错误 UI，
  用户卡在 Done 页、按钮复位但无反馈。
  - 修法：`persistSettings` 失败时设置可见 saveError 状态并在操作区展示。

- **F4（中）`src/onboarding/OnboardingShell.tsx:69` + `:119`** —
  `providerBypass` 放行离开供应商步，但 `handleFinish` 仍要求 `canCompleteOnboarding`
  （= `validateProviderStep.ok`），bypass 后 Finish 永久禁用，唯一出口是 Skip，语义误导。
  - 修法：bypass 时让 Finish 走 skipped 完成语义（持久化 + onComplete），保证有可用出口。

- **F5（中）`src/onboarding/validation.ts` `isProviderModelBindingUsable`** —
  从不校验 `baseUrl`，可用空/乱 base URL 完成引导。
  - 修法：绑定校验中加入 provider.baseUrl trim 后非空判断。

- **F6（中）`src/onboarding/OnboardingShell.tsx:47`** —
  `loadSettings` 无条件 `settingsLanguage: detectSystemLang()`，重跑引导会覆盖老用户已选语言。
  - 修法：仅当 loaded 无有效 `settingsLanguage` 时才用 `detectSystemLang()`，否则沿用 loaded。

- **F7（中）`src-tauri/src/chat/commands.rs:4218`** —
  `try_auto_compress_context_after_update` 成功分支无条件 `warning = None`，抹掉
  `compact_conversation` 设好的 `decay_warning_for(count)`（compaction.rs:1332，已由
  compute_context_state:4137 保留）。另一自动路径（commands.rs:1288）不抹，行为不一致。
  - 修法：删除该 `= None` 行（compute_context_state 已保留正确的 decay-或-None 值）。

- **F8（低）`src-tauri/src/native_tools/sandbox_exports.rs:24/28`** —
  `MAX_EXPORT_FILES_PER_RUN=16` > `DELIVERY_DIR_MAX_FILES=15`，单次运行产 16 文件时
  prune 立即吞掉本次最老的一个。
  - 修法：令 `DELIVERY_DIR_MAX_FILES >= MAX_EXPORT_FILES_PER_RUN`（统一到 16）。

- **F9（低）`src/onboarding/ProviderSetupPanel.tsx:402`** —
  `apiKeys: value.trim() ? [value] : []` 存原值（含空白）。
  - 修法：存 `[value.trim()]`。

- **F10（测试）`src-tauri/src/settings.rs:3047`** —
  `effective_side_models_auto_prefer_session_over_global_chat_default` 缺 `#[test]`，从不运行。
  - 修法：补 `#[test]`；确保断言实际执行且通过（若不通过则为真 bug，再评估）。

## Deferred（已核实为真，但本次不修）
- Gemini：单 `thoughtSignature` 盖到该轮所有无签名 functionCall（并行调用；改对需按 part
  精确配对，有回归单调用常见路径的风险）。
- 多模型：流式列瞬时错标（`groupStreamingStore` 按到达序认领占位列，持久化后自纠正）；
  `chat_set_group_selection` 丢更新竞态；regenerate-in-group UX；前后端 fan-out 判定不一致空占位列。
- Gemini：`maxOutputTokens` 无 `>0` 守卫；纯思考轮 signature 丢弃；empty-response 取消瞬间多打一次。
- SSE 逐 chunk `from_utf8_lossy` 多字节边界（预存、三适配器共有，宜单独任务统一修）。
- 调试环形缓冲不限 request body（开发功能默认关）；Grok 结果重复、L2 后 compressed_message_count=0（观感）。

## 验收标准
- [ ] F1–F10 全部按修法落地，不触碰 Deferred 项。
- [ ] `npm run lint` / `npm run typecheck` / `npm test` 全绿。
- [ ] Rust 测试通过（用 `scripts/win-cargo-test.ps1`，见 [[windows-rust-test-manifest]]）；F10 测试实际执行且通过。
- [ ] 不引入新的 provider 400 风险。
- [ ] 前端修点补/改对应 Vitest 断言（onboarding validation 等），已有 test 保持绿。
