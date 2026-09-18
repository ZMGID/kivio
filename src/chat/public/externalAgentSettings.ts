import { chatApi } from '../api'

export type {
  CcSwitchProvider,
  DetectedExternalAgent,
  DshOfficialCredential,
  ExternalCliInstallInfo,
} from '../api'
export type { NativeProviderSummary } from '../types'

/** Settings-facing operations for discovering and maintaining external agents. */
export const externalAgentSettingsApi: Pick<
  typeof chatApi,
  | 'detectExternalAgents'
  | 'detectExternalAgentModels'
  | 'dshNativeProviderDelete'
  | 'dshNativeProviderGet'
  | 'dshOfficialCredentialSave'
  | 'dshOfficialCredentialStatus'
  | 'externalCliInstall'
  | 'externalCliInstallInfo'
  | 'externalCliProviderCleanup'
> = chatApi
