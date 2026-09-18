import { chatApi } from '../api'

export type {
  PiExtensionInventory,
  PiExtensionPackage,
  PiLocalExtension,
} from '../api'

/** Settings-facing operations for Pi extension lifecycle management. */
export const piExtensionsSettingsApi: Pick<
  typeof chatApi,
  | 'piExtensionInstall'
  | 'piExtensionOpen'
  | 'piExtensionRemove'
  | 'piExtensionSetEnabled'
  | 'piExtensionsInventory'
  | 'piExtensionsOpenDir'
  | 'piExtensionUpdate'
> = chatApi
