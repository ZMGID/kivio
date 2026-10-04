# Model database refresh — 2026-09-24

Official vendor documentation was checked against the September 12 catalog. Prices below are USD per million tokens, in input / output / cached-input order. The shared baseline remains `src/data/modelDatabase.json`; explicit user overrides retain priority. This update does not enable models on users' provider accounts or change saved model selections.

## Added entries

| API model ID | Stored context / output | Token pricing | Source |
| --- | --- | --- | --- |
| `gpt-6-sol` | 256,000 / 128,000 | 2 / 10 / 0.20 | [OpenAI](https://developers.openai.com/api/docs/models/gpt-6-sol) |
| `gpt-6-luna` | 256,000 / 128,000 | 0.10 / 0.50 / 0.01 | [OpenAI](https://developers.openai.com/api/docs/models/gpt-6-luna) |
| `claude-opus-5-5` | 1,000,000 / 128,000 | 4 / 20 / 0.20 | [Anthropic](https://platform.claude.com/docs/en/models/opus-5-5/overview) |
| `mimo-v2.6-pro` | 1,048,576 / 131,072 | 0.435 / 0.87 / 0.0036 | [Xiaomi](https://mimo.mi.com/models/en-US/mimo-v2.6-pro) |
| `mimo-v2.6-flash` | 1,048,576 / 131,072 | 0.14 / 0.28 / 0.0028 | [Xiaomi](https://mimo.mi.com/models/en-US/mimo-v2.6-flash) |
| `grok-4.7` | 500,000 / unspecified | 2 / 6 / 0.50 | [xAI model](https://docs.x.ai/developers/grok-4-7), [pricing](https://docs.x.ai/developers/pricing) |
| `glm-5.3-flashx` | 1,000,000 / 131,072 | 0.37 / 1.25 / 0.075 | [Z.ai model](https://docs.z.ai/guides/vlm/glm-5.3-flash), [pricing](https://docs.z.ai/guides/overview/pricing) |
| `qwen3.8-omni-flash` | 1,000,000 / 131,072 | 0.15 / 0.47 / 0.016 | [Alibaba model](https://help.aliyun.com/zh/model-studio/qwen3-8-omni-flash), [pricing](https://www.alibabacloud.com/help/en/model-studio/model-pricing) |

The Claude database key uses `claude-opus-5.5`, following existing separator normalization; requests still use the selected provider's ID. New IDs and provider-prefixed variants resolve independently of older family members.

## Limits and protocol details

- OpenAI documents 1,050,000-token contexts. The project intentionally retains its 256K GPT-5.6/GPT-6 default; users can override it. Sol and Luna expose low/medium/high/xhigh/max; the existing Off control handles API `none`. Reasoning with tools requires Responses; Chat Completions only supports their function calls with effort `none`. These are catalog entries, not a change to provider routing.
- Opus 5.5 supports the five effort levels and always-on adaptive thinking. The existing Claude profile now avoids sending `thinking.type=disabled` for Opus 5.5+, as it already does for always-on Fable models. Opus 5 and Sonnet 5 behavior is preserved. An actual request-body regression covers Off, temperature removal, and each effort. The product's existing default effort is unchanged. [Effort documentation](https://platform.claude.com/docs/en/build-with-claude/effort).
- MiMo V2.6 Pro/Flash have image and video input. Their published 1M/128K limits use the existing MiMo family's 1,048,576/131,072 catalog convention. The reviewed pages document thinking on/off but no named effort levels, so an explicit empty effort list prevents invented low/medium/high parameters.
- Grok 4.7 documents no separate text output cap. `maxOutput: 0` means unspecified in the existing catalog; normal application/provider output defaults still apply. Rates above use short context; at 200K input tokens the token rates double. Grok 4.7 Fast is excluded because it is not available on the public xAI API.
- GLM FlashX has the same documented parameter controls as GLM-5.3, including low/high/max, and supports video. Qwen Omni accepts low/medium/high/xhigh/max; high/max are compatibility aliases for xhigh. Qwen prices use the Singapore international region; other regions differ. [Qwen controls](https://help.aliyun.com/zh/model-studio/qwen-omni).
- The schema does not represent audio capabilities. Multimodal text-output models are included for their supported text/image/video use; audio-only and realtime endpoints are excluded.

## Existing entries refreshed

- Claude Sonnet 5: pricing corrected to 2 / 10 / 0.20. [Official model page](https://platform.claude.com/docs/en/models/sonnet-5/overview).
- GLM-5.3 Flash: expired launch pricing replaced by 0.15 / 0.50 / 0.03; video capability added. Sources are the Z.ai pages above.
- MiMo V2.5 Pro: cache price 0.0036 and web-search capability added; it remains text-only. MiMo V2.5: prices 0.14 / 0.28 / 0.0028 and video/web-search capability added. Both use an empty named-effort list. Sources: [V2.5 Pro](https://mimo.mi.com/models/en-US/mimo-v2.5-pro), [V2.5](https://mimo.mi.com/models/en-US/mimo-v2.5).

## Survey boundaries

The checked [DeepSeek](https://api-docs.deepseek.com/quick_start/pricing/) and [MiniMax](https://platform.minimax.io/docs/guides/pricing-paygo) core API models/prices were already represented. [Gemini's release notes](https://ai.google.dev/gemini-api/docs/changelog) mainly add voice/TTS and an Antigravity agent endpoint after this catalog's previous snapshot; those endpoints need different transports and were not added as ordinary chat models. This was a focused current-model refresh, not a re-audit of every historical entry or a paid live inference test.

## Follow-up — 2026-10-04

The catalog now contains 335 model entries, up from 322. Thirteen entries were added after checking official vendor pages and live OpenRouter model/endpoint metadata. Prices in the table retain the existing USD-per-million-token convention; “unspecified” does not mean free. Saved provider model selections, enabled models, and explicit user overrides are unchanged.

| Catalog model ID | Stored context / output | Input / output / cached-input pricing | Source |
| --- | --- | --- | --- |
| `gpt-6.1-sol` | 256,000 / 128,000 | 2 / 10 / 0.10 | [OpenAI](https://developers.openai.com/api/docs/models/gpt-6.1-sol) |
| `openai/gpt-6.1-sol-pro` | 256,000 / 128,000 | 2 / 10 / 0.10 | [OpenRouter endpoint](https://openrouter.ai/api/v1/models/openai/gpt-6.1-sol-pro/endpoints) |
| `claude-sonnet-5.5` | 1,000,000 / 128,000 | 2 / 10 / 0.20 | [Anthropic](https://platform.claude.com/docs/en/models/sonnet-5-5/overview) |
| `minimax-m3.1-flash-preview` | 1,000,000 / 524,288 | unspecified | [MiniMax API](https://platform.minimax.io/docs/api-reference/text-chat-openai.md), [model limits](https://platform.minimax.io/docs/guides/models-intro) |
| `inclusionai/ling-3.1-flash` | 262,144 / 32,768 | 0 / 0 / unspecified | [OpenRouter endpoint](https://openrouter.ai/api/v1/models/inclusionai/ling-3.1-flash/endpoints) |
| `apodex/apodex-1.1-mini:free` | 262,144 / 235,929 | 0 / 0 / unspecified | [OpenRouter endpoint](https://openrouter.ai/api/v1/models/apodex/apodex-1.1-mini:free/endpoints) |
| `unbiased/pareto-26.10-preview` | 1,048,576 / 131,072 | 0.80 / 3.20 / 0.03 | [OpenRouter endpoint](https://openrouter.ai/api/v1/models/unbiased/pareto-26.10-preview/endpoints) |
| `perceptron/perceptron-mk1.5` | 36,864 / 8,192 | 0.15 / 1.50 / unspecified | [OpenRouter endpoint](https://openrouter.ai/api/v1/models/perceptron/perceptron-mk1.5/endpoints) |
| `fireworks/ember-1` | 1,048,576 / 943,718 | 3 / 15 / 0.30 | [OpenRouter endpoint](https://openrouter.ai/api/v1/models/fireworks/ember-1/endpoints) |
| `stepfun/step-5-preview` | 1,048,576 / 1,048,576 | unspecified in USD | [Alibaba model](https://help.aliyun.com/zh/model-studio/step-5-preview), [API](https://help.aliyun.com/zh/model-studio/stepfun) |
| `qwen-image-2.1-pro` | unspecified / unspecified | per-image billing; no token rate stored | [Alibaba](https://help.aliyun.com/zh/model-studio/qwen-image-2-1-pro) |
| `embed-v5.0-pro` | 128,000 / embedding | 0.12 / 0 / unspecified | [Cohere model IDs](https://docs.cohere.com/v2/docs/models), [release and pricing](https://cohere.com/blog/embed-5) |
| `embed-v5.0-fast` | 128,000 / embedding | 0.08 / 0 / unspecified | [Cohere model IDs](https://docs.cohere.com/v2/docs/models), [release and pricing](https://cohere.com/blog/embed-5) |

### Access, controls, and pricing boundaries

- GPT-6.1 Sol retains the project's 256K GPT default, including the OpenRouter Pro alias. The published total context is 1,050,000 tokens, with up to 922,000 input and 128,000 output tokens. Sol requires reasoning: `none` and `minimal` are not supported; select a supported thinking level. Direct tool use requires Responses, not Chat Completions. The stored rates are short-context rates; prompts above 272K tokens have higher rates. Pro is a reasoning mode on the direct OpenAI API, not a separate direct `gpt-6.1-sol-pro` model ID. The fully namespaced catalog entry refers only to OpenRouter's advertised alias.
- Sonnet 5.5 uses API ID `claude-sonnet-5-5`; the dotted catalog key follows existing normalization. Its Off request now sends only `thinking: {"type": "between_tools"}` and no effort override, retaining the upstream default `high`. It turns off up-front thinking, not tool progress thinking. `between_tools` rejects `display`, manual budgets, and `xhigh`/`max`; enabled requests use adaptive thinking with the selected effort. Sonnet 5 still sends `disabled`, and Opus 5.5/Fable remain always-on. Forced tool use is not supported by Sonnet 5.5. [Breaking changes](https://platform.claude.com/docs/en/models/sonnet-5-5/whats-new-sonnet-5-5).
- MiniMax M3.1 Flash Preview is available only through M Plan / MiniMax Code, not ordinary pay-as-you-go access. The selected API ID is `MiniMax-M3.1-Flash-Preview`; matching is case-insensitive. Image/video input and low/medium/high/xhigh/max are documented. Thinking is mandatory, so Off/`none` is not supported. No unpublished pay-as-you-go token price is invented.
- Hosted entries retain their full provider namespaces and the Apodex `:free` suffix. The checked endpoints reported status `0`; availability and free pricing can change. Ling and Apodex publish reasoning support but no named effort list, so the list is explicitly empty. Pareto exposes no reasoning control in its endpoint contract, so no reasoning knob is advertised. Perceptron stores the supported low/medium/high UI levels; the API also accepts minimal and none. Ember stores low/high/max. [Live catalog, including effort metadata](https://openrouter.ai/api/v1/models).
- Step 5 Preview's verified endpoint is the Beijing Alibaba workspace OpenAI-compatible API. Its prices are published in CNY (7 input / 20 output / 0.35 cached input per million tokens), so no guessed USD conversion is stored. Thinking is off by default and requires the endpoint-specific `enable_thinking: true` extra request-body field; supported efforts are low/medium/high. This refresh does not add provider routing or private-parameter automation.
- Qwen Image 2.1 Pro accepts text and reference images and produces images, not chat text. Context/output limits are unpublished and retain the existing zero sentinel. Published Beijing pricing is CNY 0.25 per image; the Singapore price is CNY 0.283404 per image. Neither is entered as USD token pricing.
- Embed 5 Pro/Fast support multimodal, multilingual embeddings with a default 2,048 dimensions and optional 256/512/768/1024/1536/2048 dimensions. The stored input price is for text; image input is separately priced at USD 0.40 per million tokens. This adds catalog metadata, not a new Cohere-native embedding transport or a multimodal indexing workflow.

### Existing limits corrected and verification

- MiniMax M3: context corrected from 1,048,576 to the documented 1,000,000, maximum output from 512,000 to 524,288, and video input added. MiniMax M2.7 / Highspeed: maximum output corrected from 131,072 to 204,800. [Official model table](https://platform.minimax.io/docs/guides/models-intro).
- Catalog matching/import tests: 65 passed. Anthropic request-body tests: 41 passed; model-metadata tests: 30 passed. TypeScript type checking passed. A Sonnet 5.5 request-body regression failed with the old `disabled` field before the fix and passed with `between_tools`; obsolete profile-field-only assertions were removed.
- A disposable Vite harness mounted the actual model detail drawer with the application styles. All 13 selected IDs, names, context/output values, capabilities, token prices, and named effort controls were exercised; a saved GPT context override survived switching models and did not alter the Pro entry. The visible Sonnet drawer was captured. No paid inference or native desktop/provider-access smoke was performed.
- Batch-only Claude aliases, automatic routers with sentinel prices, retired IDs, and unconfirmed release names were excluded. Existing historical entries were not mass-removed.
