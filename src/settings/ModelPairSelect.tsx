import { type ModelProvider } from '../api/tauri'
import { Select } from './components'
import { buildModelPairOptions, modelPairValue, parseModelPairValue, type Platform } from './utils'

interface ModelPairSelectProps {
  providerId: string
  model: string
  providers: ModelProvider[]
  platform: Platform
  onChange: (providerId: string, model: string) => void
  inheritLabel?: string
  className?: string
}

export function ModelPairSelect({
  providerId,
  model,
  providers,
  platform,
  onChange,
  inheritLabel,
  className = 'w-52',
}: ModelPairSelectProps) {
  const options = [
    ...(inheritLabel ? [{ value: modelPairValue('', ''), label: inheritLabel }] : []),
    ...buildModelPairOptions(providers, platform),
  ]

  return (
    <Select
      className={className}
      value={modelPairValue(providerId, model)}
      onChange={(value) => {
        const [nextProviderId, nextModel] = parseModelPairValue(value)
        onChange(nextProviderId, nextModel)
      }}
      options={options}
    />
  )
}
