import modelDatabase from './modelDatabase.json'
import type { ModelInfo } from '../api/tauri'

type DbEntry = {
  displayName: string
  contextWindow: number
  maxOutput: number
  capabilities: {
    vision?: boolean
    functionCalling?: boolean
    reasoning?: boolean
    streaming?: boolean
    webSearch?: boolean
    imageGeneration?: boolean
    embedding?: boolean
  }
  dimensions?: number
  multilingual?: boolean
  pricing?: {
    input: number
    output: number
    cachedInput?: number
  }
}

const raw = modelDatabase as Record<string, unknown>
// eslint-disable-next-line @typescript-eslint/no-unused-vars
const { _meta: _, ...dbEntries } = raw
const db = dbEntries as unknown as Record<string, DbEntry>
const dbKeys = Object.keys(db)

/**
 * 从内置数据库匹配模型信息
 *
 * 匹配优先级：
 * 1. 精确匹配
 * 2. 前缀匹配（最长）
 * 3. 包含匹配（最长）
 *
 * 自动处理 OpenRouter 风格 "provider/model" → 去掉前缀再匹配
 */
export function matchModel(modelName: string): ModelInfo | null {
  if (!modelName.trim()) return null

  // 去掉 provider/ 前缀（如 "openai/gpt-4o" → "gpt-4o"）
  const name = modelName.toLowerCase().trim()
  const stripped = name.includes('/') ? name.split('/').pop()! : name

  // 1. 精确匹配。先查完整 ID，允许数据库保存 OpenRouter 风格的 provider/model 专用条目。
  if (db[name]) {
    return toModelInfo(db[name])
  }
  if (db[stripped]) {
    return toModelInfo(db[stripped])
  }

  const candidates = name === stripped ? [stripped] : [name, stripped]

  // 2. 前缀匹配（最长 key 优先）
  const prefixMatches = dbKeys
    .filter(key => candidates.some(candidate => candidate.startsWith(key) && key.length < candidate.length))
    .sort((a, b) => b.length - a.length)
  if (prefixMatches.length > 0) {
    return toModelInfo(db[prefixMatches[0]])
  }

  // 3. 包含匹配（最长 key 优先）
  const containsMatches = dbKeys
    .filter(key => candidates.some(candidate => candidate.includes(key) && key !== candidate))
    .sort((a, b) => b.length - a.length)
  if (containsMatches.length > 0) {
    return toModelInfo(db[containsMatches[0]])
  }

  const imageGenerationFallback = matchKnownImageGenerationModel(stripped)
  if (imageGenerationFallback) {
    return imageGenerationFallback
  }

  return null
}

/**
 * 合并模型信息：数据库默认值 + 用户覆盖
 * 用户覆盖的字段优先，未覆盖的字段用数据库默认
 */
export function resolveModelInfo(
  modelName: string,
  overrides?: Record<string, ModelInfo>,
): ModelInfo {
  const defaults = matchModel(modelName)
  const override = overrides?.[modelName]

  if (!defaults && !override) return {}
  if (!defaults) return override!
  if (!override) return defaults

  return {
    displayName: override.displayName ?? defaults.displayName,
    contextWindow: override.contextWindow ?? defaults.contextWindow,
    maxOutput: override.maxOutput ?? defaults.maxOutput,
    capabilities: {
      vision: override.capabilities?.vision ?? defaults.capabilities?.vision,
      functionCalling: override.capabilities?.functionCalling ?? defaults.capabilities?.functionCalling,
      reasoning: override.capabilities?.reasoning ?? defaults.capabilities?.reasoning,
      streaming: override.capabilities?.streaming ?? defaults.capabilities?.streaming,
      webSearch: override.capabilities?.webSearch ?? defaults.capabilities?.webSearch,
      imageGeneration: override.capabilities?.imageGeneration ?? defaults.capabilities?.imageGeneration,
      embedding: override.capabilities?.embedding ?? defaults.capabilities?.embedding,
    },
    dimensions: override.dimensions ?? defaults.dimensions,
    multilingual: override.multilingual ?? defaults.multilingual,
    pricing: {
      input: override.pricing?.input ?? defaults.pricing?.input,
      output: override.pricing?.output ?? defaults.pricing?.output,
      cachedInput: override.pricing?.cachedInput ?? defaults.pricing?.cachedInput,
    },
  }
}

/**
 * 判断模型是否有任何已知信息（数据库匹配或用户覆盖）
 */
export function hasModelInfo(
  modelName: string,
  overrides?: Record<string, ModelInfo>,
): boolean {
  return !!(matchModel(modelName) || overrides?.[modelName])
}

function toModelInfo(entry: DbEntry): ModelInfo {
  return {
    displayName: entry.displayName,
    contextWindow: entry.contextWindow,
    maxOutput: entry.maxOutput,
    capabilities: {
      vision: entry.capabilities.vision ?? false,
      functionCalling: entry.capabilities.functionCalling ?? false,
      reasoning: entry.capabilities.reasoning ?? false,
      streaming: entry.capabilities.streaming ?? false,
      webSearch: entry.capabilities.webSearch ?? false,
      imageGeneration: entry.capabilities.imageGeneration ?? false,
      embedding: entry.capabilities.embedding ?? false,
    },
    dimensions: entry.dimensions,
    multilingual: entry.multilingual,
    pricing: {
      input: entry.pricing?.input,
      output: entry.pricing?.output,
      cachedInput: entry.pricing?.cachedInput,
    },
  }
}

function matchKnownImageGenerationModel(modelName: string): ModelInfo | null {
  if (/^gpt-image-[a-z0-9][a-z0-9.-]*$/.test(modelName)) {
    return knownImageGenerationModelInfo(modelName, true)
  }
  if (/^dall-e-[a-z0-9][a-z0-9.-]*$/.test(modelName)) {
    return knownImageGenerationModelInfo(modelName, modelName !== 'dall-e-3')
  }
  return null
}

function knownImageGenerationModelInfo(modelName: string, vision: boolean): ModelInfo {
  return {
    displayName: titleCaseModelName(modelName),
    contextWindow: 0,
    maxOutput: 0,
    capabilities: {
      vision,
      functionCalling: false,
      reasoning: false,
      streaming: false,
      webSearch: false,
      imageGeneration: true,
    },
    pricing: {
      input: 0,
      output: 0,
    },
  }
}

function titleCaseModelName(modelName: string): string {
  return modelName
    .split('-')
    .map((part) => {
      if (part === 'gpt') return 'GPT'
      if (part === 'dall') return 'DALL'
      if (part === 'e') return 'E'
      return part.length === 0 ? part : part[0].toUpperCase() + part.slice(1)
    })
    .join(' ')
}
