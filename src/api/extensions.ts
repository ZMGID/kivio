import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
export const extensionsApi = {
  list: () => invoke<string[]>('extensions_list'),
  setEnabled: (id: string, enabled: boolean) => invoke<string[]>('extensions_set_enabled', { id, enabled }),
  subscribe: (changed: () => void) => listen('extensions-changed', changed),
}
