import { chatApi } from '../api'

export type {
  DshPluginEntry,
  DshPluginSettingsPatch,
  DshPluginSettingsSnapshot,
} from '../api'

/** Settings-facing operations for DSH plugin configuration and inventory. */
export const dshPluginSettingsApi: Pick<
  typeof chatApi,
  | 'dshOpenSettingsFile'
  | 'dshPluginInventory'
  | 'dshPluginSettingsGet'
  | 'dshPluginSettingsSave'
> = chatApi
