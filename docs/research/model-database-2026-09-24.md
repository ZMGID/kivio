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
