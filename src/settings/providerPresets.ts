// Provider 预设：常用 OpenAI 兼容 endpoint 的一键填充模板。
// 添加新预设只需在 PROVIDER_PRESETS 末尾追加一项。

export type ProviderPreset = {
  /** chip 上显示的名字，也作为新 provider 的 name 字段（用户可改） */
  name: string
  /** OpenAI 兼容的 base URL（含 /v1） */
  baseUrl: string
  /** 默认启用 + 加入 availableModels 的几个典型模型，让用户填完 key 就能直接选 */
  defaultModels: string[]
}

export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    name: 'DeepSeek',
    baseUrl: 'https://api.deepseek.com/v1',
    defaultModels: ['deepseek-chat', 'deepseek-reasoner'],
  },
  {
    name: 'OpenRouter',
    baseUrl: 'https://openrouter.ai/api/v1',
    defaultModels: ['anthropic/claude-sonnet-4.5', 'openai/gpt-4o-mini'],
  },
  {
    name: 'SiliconFlow',
    baseUrl: 'https://api.siliconflow.cn/v1',
    defaultModels: ['Qwen/Qwen2.5-72B-Instruct', 'deepseek-ai/DeepSeek-V3'],
  },
  {
    name: 'GLM',
    baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
    defaultModels: ['glm-4-plus', 'glm-4v-plus'],
  },
  {
    name: 'Ollama',
    baseUrl: 'https://ollama.com/v1',
    defaultModels: ['gpt-oss:120b', 'llama3.3:70b'],
  },
]
