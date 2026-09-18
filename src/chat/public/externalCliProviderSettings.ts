import { chatApi } from '../api'

export type { CcSwitchProvider, CcSwitchScan } from '../api'

/** Settings-facing operations for external CLI provider configuration and import. */
export const externalCliProviderSettingsApi: Pick<
  typeof chatApi,
  'externalCliFetchRelayModels' | 'externalCliScanCcSwitch'
> = chatApi
