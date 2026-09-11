# Model database refresh — 2026-09-12

This refresh covers releases and material catalog changes discovered after the previous 2026-09-06 snapshot. Vendor documentation is preferred; OpenRouter's live catalog is used only for OpenRouter-specific IDs, limits, and prices.

## Added models

### DeepSeek V4.1 Flash

- Official hosted API ID: `deepseek-flash`.
- 1,048,576-token context, 384,000-token maximum output, image input, tool calls, streaming, and reasoning.
- Peak pricing is stored as the flat database baseline: $0.30 uncached input, $0.006 cached input, and $1.20 output per million tokens. Off-peak rates are half of those values.
- The hosted API's native efforts are `low`, `high`, and `max`; it also accepts `xhigh` and maps it to `high`. Kivio preserves `xhigh` for compatibility with its existing Pi model-level contract. The open-weight model supports a continuous 1–100 control.
- `deepseek-v4-flash` and `deepseek-v4-flash-vision-exp` now temporarily route to V4.1 Flash. Their entries were updated to the served model's visual capability and pricing while retaining explicit legacy-alias labels.

Sources: [DeepSeek release](https://deepseek.com/en/news/deepseek-v4-1-flash/), [official pricing and model table](https://api-docs.deepseek.com/quick_start/pricing/), [thinking-mode mapping](https://api-docs.deepseek.com/guides/thinking_mode/), [official open-weight model card](https://huggingface.co/deepseek-ai/DeepSeek-V4.1-Flash).

### GPT Image 2.5

- Added `gpt-image-2.5-flare` and `gpt-image-2.5-sunburst`, plus their `-2026-09-08` snapshots.
- Both accept text and image input, produce image output, and support generation and editing through the Images API or the Responses API image-generation tool.
- The catalog's generic token-price fields store text input ($5), cached text input ($1.25), and image output ($30) per million tokens. OpenAI separately prices image input at $8 / $2 cached per million image tokens; the current schema cannot express two input modalities independently.

Sources: [OpenAI launch](https://openai.com/index/introducing-chatgpt-images-2-5/), [Flare model page](https://developers.openai.com/api/docs/models/gpt-image-2.5-flare), [Sunburst model page](https://developers.openai.com/api/docs/models/gpt-image-2.5-sunburst).

### Other current additions

- `gpt-5.6-cyber`: 128K output, image input, tools, and $12.50 / $75 token pricing. It requires separate Daybreak approval. Kivio intentionally caps its default context at 256K.
- `mercury-2.5`: 260K context, reasoning and tool use. The stored $0.04 / $0.15 prices are the current launch promotion; list pricing is $0.20 / $0.75.
- `ling-3.0-flash` and `ling-3.0-flash-vl`: official open-weight Ling 3.0 text and native vision/video variants. The VL entry is marked for visual input, not image generation.
- `nex-n2.5-mini` and `nex-n2.5-pro`: multimodal agent models. The currently listed OpenRouter routes are free, so token prices are zero.
- `fugu-max` and `fugu-ultra-v2`: Sakana orchestration models released September 11. Fugu Max's official price is $2 / $6; the remaining channel limits and prices come from OpenRouter's live catalog.

Sources: [GPT-5.6 Cyber](https://developers.openai.com/api/docs/models/gpt-5.6-cyber), [Mercury 2.5](https://www.inceptionlabs.ai/blog/introducing-mercury-2-5), [Ling 3.0 Flash](https://huggingface.co/inclusionAI/Ling-3.0-flash), [Ling 3.0 Flash VL](https://huggingface.co/inclusionAI/Ling-3.0-flash-VL), [Nex N2.5](https://nex-agi.com/), [Fugu Max and Ultra v2](https://sakana.ai/fugu-max-release/), [OpenRouter live model catalog](https://openrouter.ai/api/v1/models).

## Corrected existing entries

OpenAI's current model catalog documents 1.05M-token contexts and reduced prices for the GPT-5.6 family. Kivio intentionally uses a conservative 256K system default for GPT-5.6 and GPT-6 models; users may override it per model. Updated prices:

| Model | Input | Cached input | Output |
| --- | ---: | ---: | ---: |
| `gpt-5.6` / `gpt-5.6-sol` | $4.00 | $0.40 | $20.00 |
| `gpt-5.6-terra` | $2.00 | $0.20 | $12.00 |
| `gpt-5.6-luna` | $0.20 | $0.02 | $1.20 |

Source: [OpenAI model catalog](https://developers.openai.com/api/docs/models).

## Deliberate exclusions

- `GPT-Live-1` was not added because Kivio's model metadata does not currently represent audio input/output or realtime voice capabilities; marking it as an ordinary chat model would be misleading.
- Nex N2.5 Max was not added because the surveyed live API catalog did not expose a callable route or stable price, even though the open-weight family announcement describes it.
- Experimental quantizations and community aliases were excluded unless a first-party model card or a live provider catalog exposed a stable ID.
