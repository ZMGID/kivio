import { type ModelProvider } from '../api/tauri'
import { Select } from './components'
import { buildModelPairOptions, modelPairValue, parseModelPairValue } from './utils'

interface ModelPairSelectProps {
  providerId: string
  model: string
  providers: ModelProvider[]
  onChange: (providerId: string, model: string) => void
  inheritLabel?: string
  offOption?: { label: string; selected: boolean; onSelect: () => void }
  className?: string
  ariaLabel?: string
  /** 可选：只保留满足谓词的模型（如生图模型仅列 imageGeneration=true）。 */
  filterModel?: (provider: ModelProvider, model: string) => boolean
}

export function ModelPairSelect({
  providerId,
  model,
  providers,
  onChange,
  inheritLabel,
  offOption,
  className = 'w-52',
  ariaLabel,
  filterModel,
}: ModelPairSelectProps) {
  const filtered = buildModelPairOptions(providers, filterModel)
  // 当前已选模型若被筛掉（如老配置选了非生图模型），仍补进选项，避免显示空白。
  const currentValue = modelPairValue(providerId, model)
  const hasCurrent = !providerId && !model
    ? true
    : filtered.some(option => option.value === currentValue)
  const currentLabel = providers.find(p => p.id === providerId)?.name
  const options = [
    ...(offOption ? [{ value: 'off', label: offOption.label }] : []),
    ...(inheritLabel ? [{ value: modelPairValue('', ''), label: inheritLabel }] : []),
    ...filtered,
    ...(hasCurrent
      ? []
      : [{ value: currentValue, label: `${currentLabel ?? providerId} - ${model}`, title: `${currentLabel ?? providerId} - ${model}` }]),
  ]

  return (
    <Select
      className={className}
      ariaLabel={ariaLabel}
      value={offOption?.selected ? 'off' : currentValue}
      onChange={(value) => {
        if (value === 'off' && offOption) {
          offOption.onSelect()
          return
        }
        const [nextProviderId, nextModel] = parseModelPairValue(value)
        onChange(nextProviderId, nextModel)
      }}
      options={options}
    />
  )
}
