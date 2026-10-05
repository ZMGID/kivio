# Kivio Chat Runtime Model Contract

The Chat runtime owns orchestration: conversation state, prompt construction, tool loop,
cancellation, persistence, and Tauri events.

Provider adapters own provider JSON and wire protocols. Runtime code should pass
`GenerateRequest` to a `LanguageModelProvider` and consume `GenerateOutput` plus
`StreamPart` events. OpenAI-compatible and Anthropic Messages are peer providers.

Rules:

- Runtime and tool loop code should not inspect OpenAI `choices`, Anthropic `content`
  blocks, SSE event names, or provider-specific headers.
- Provider adapters may use `serde_json::Value` freely at their wire boundary.
- Storage can keep legacy `api_messages` for compatibility, but new replay logic should
  prefer canonical model messages when available.
- Realtime Tauri payloads are generated from the Rust `chat-protocol` contract.
  are UI contracts, not provider contracts.
- Model-generated images are protocol-agnostic on the contract: adapters parse each
  wire format's output-image shape (Gemini `inlineData`, …) into `GenerateOutput.images`
  (and stream `StreamPart::ImageData`); runtime never inspects provider image JSON.
- `StreamPart::ContextUsage` reports the current main request's normalized input and
  output as soon as the provider supplies them. The meter uses input + output, never
  the accumulated bill or local estimate. Anthropic input includes its disjoint cache
  reads/writes; OpenAI input already includes cache. Partial Anthropic usage replaces
  supplied cumulative fields without erasing omitted fields.
  Snapshots use `provider_context_reported` with `reported_context_tokens`; old
  prompt-only/mixed/estimated sources are not promoted to this measurement.
- Categories are captured from the prepared main request, including its filtered tools,
  rather than reconstructed by the stats endpoint. `chars` measure UTF-16 content/
  serialized JSON length, excluding image payloads. At the actual request boundary,
  image estimates use bounded headers (about 64 KiB decoded) and are added to the
  conversation category, never to character shares. Chat Completions, Responses,
  and native image blocks use the same estimator and are counted once.
  Independent token estimates are never scaled to usage.
  The UI labels character shares and uses them only to partition the occupied bar.
  Image sizing follows [OMP's dimension/detail approach](https://github.com/can1357/oh-my-pi/blob/898b09d32f147887a2242cf5ec9a1967bcac8873/packages/agent/src/image-tokens.ts),
  with model-specific parameters checked against the
  [OpenAI vision guide](https://developers.openai.com/api/docs/guides/images-vision/)
  on 2026-10-05, not a universal multiplier. Pixel-dimension fitting precedes patch
  budgeting. `low`, `high`, `original`, and `auto` retain each model's semantics.
  Explicit image `detail` survives normalization, stored model messages, and both
  OpenAI wire formats. Older messages omit it and keep the provider's default;
  Anthropic and Gemini do not receive this OpenAI-specific field.
  Unknown OpenAI image dimensions use the selected detail's budget (or accepted
  patch limit for original-detail models without a resizing budget); tile models
  use their fixed low-detail cost or eight-tile upper estimate. Claude uses its
  model cap; Gemini retains the 1,600-token unknown-size heuristic, not a guaranteed
  upper bound. No remote image fetch is performed just for accounting.
  These are display estimates; provider reports still own the meter, and this
  change does not alter the internal compaction budget or trigger policy.
- `measurementSeq` orders both live events and persisted snapshots; lower sequences
  cannot overwrite newer measurements. `lifecycleId` advances on invalidation.
  Model changes and history rewrites clear the meter; switching back cannot revive it.
  Selecting another answer in a multi-answer group invalidates memory and persisted
  measurements together, rejects late reports, and recalculates selected-branch cache
  usage. Selecting the same answer again does not invalidate its measurement.
  Missing request usage stays unknown. Stream deltas do not write conversation files.
- Completed main requests persist complete normalized input/cache-read pairs,
  separately from billing usage. Cache hit rate is `sum(cache_read) / sum(input)`
  over selected-branch main replies since the last clear and the current run.
  Like [ZCode's active-history cache accounting](https://github.com/zai-org/ZCode/blob/29628c9acdb81b703bbd4080c207a0e7ce5e276e/apps/zcode-cli/packages/core/src/runtime/methods/turn-model-step-usage.ts#L66-L104),
  branch membership determines which stored replies contribute. In-flight cache
  callbacks additionally require the currently bound run ID, just like token
  reports: an old run cannot replace a newer branch's cache pair after invalidation.
  A request missing either field contributes neither numerator nor denominator.
  Auxiliary summarization and subagents are excluded. Unknown rates and rates
  below ZCode's 78% display threshold remain hidden.
