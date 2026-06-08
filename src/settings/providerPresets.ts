// Presets only prefill provider metadata. Models are fetched from the provider API
// and explicitly enabled by the user.

export type ProviderPreset = {
  name: string
  /** OpenAI-compatible base URL, usually including /v1. */
  baseUrl: string
}

export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    name: 'DeepSeek',
    baseUrl: 'https://api.deepseek.com/v1',
  },
  {
    name: 'OpenRouter',
    baseUrl: 'https://openrouter.ai/api/v1',
  },
  {
    name: 'SiliconFlow',
    baseUrl: 'https://api.siliconflow.cn/v1',
  },
  {
    name: 'GLM',
    baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
  },
  {
    name: 'Ollama',
    baseUrl: 'https://ollama.com/v1',
  },
]
