# 执行计划 — 修复审查发现

这批是各自独立的局部修复，无跨切设计。按"后端 Rust → 前端 TS → 验证"分组执行，
每组内尽量单文件收敛，改一处补一处测试。

## 后端 Rust

- [ ] **F1** `chat/model/openai.rs:~429` 删除 `body["promptCacheKey"] = ...` 行，保留 `prompt_cache_key`。
      更新/新增 `build_openai_body` 断言：body 不含 `promptCacheKey`，仍含 `prompt_cache_key`。
- [ ] **F2** `chat/commands.rs:~4497` `group_answer_excluded_from_context` 回退查找加
      `m.stream_outcome.as_deref() != Some("error")`；全组皆 error 时才退回第一条。加/改单测。
- [ ] **F7** `chat/commands.rs:~4218` 删除 `conversation.context_state.warning = None;`（成功分支）。
- [ ] **F8** `native_tools/sandbox_exports.rs:28` `DELIVERY_DIR_MAX_FILES` 16（≥ per-run 16）；
      调整 line 756/765 现有测试的期望值。
- [ ] **F10** `settings.rs:3047` 补 `#[test]`；跑测试确认通过。

## 前端 TS

- [ ] **F5** `onboarding/validation.ts` `isProviderModelBindingUsable` 加 `provider.baseUrl?.trim()` 非空判断；
      更新 `validation.test.ts` 覆盖空 baseUrl → ok:false。
- [ ] **F9** `onboarding/ProviderSetupPanel.tsx:402` 存 `[value.trim()]`。
- [ ] **F6** `onboarding/OnboardingShell.tsx:47` 改为 `settingsLanguage: loaded.settingsLanguage || detectSystemLang()`
      （沿用已有值；仅首次为空时探测）。
- [ ] **F3** `onboarding/OnboardingShell.tsx` 加 `saveError` state；`persistSettings` 失败 setSaveError(msg)，
      成功清空；在 Done 步/操作区展示。i18n 补 zh/en 文案。
- [ ] **F4** `onboarding/OnboardingShell.tsx` bypass 出口：`handleFinish` 在 `providerBypass && !canComplete` 时
      按 `skipped` 持久化并 `onComplete`（或复用 handleSkip 路径）；确保按钮不再永久禁用。
      检查 `DoneStep` Finish 禁用条件同步放开。

## 验证（每组后 + 收尾）
- [ ] `npm run lint`（--max-warnings 0）
- [ ] `npm run typecheck`
- [ ] `npm test`（重点：onboarding validation、compactionBoundary、MessageGroup 等）
- [ ] Rust：`powershell -File scripts/win-cargo-test.ps1`（覆盖 openai body / group-context / settings side-model / sandbox_exports）
- [ ] 逐条回看 diff，确认未误伤 Deferred 项与安全护栏

## 回滚点
- 后端与前端相互独立，可按 F 编号单独 revert。
- F4 若引入引导流程回归，优先回退到"bypass=Skip 语义"最小实现。
