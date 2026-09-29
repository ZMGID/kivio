import { invoke } from '@tauri-apps/api/core'

export type PluginPackage = {
  id: string
  name: string
  description: string
  version: string | null
  format: string
  source: string
  revision: string | null
  enabled: boolean
  components: Record<string, number>
  diagnostics: string[]
  marketplace?: { source: string; name: string; plugin: string }
}

export type PluginDetails = {
  author: string | null
  version: string | null
  homepage: string | null
  license: string | null
  groups: { kind: 'mcp' | 'skills' | 'commands' | 'agents' | 'hooks'; items: { name: string; description: string }[] }[]
  diagnostics: string[]
}

export const packageApi = {
  describe: (id: string) => invoke<PluginDetails>('plugin_packages_describe', { id }),
  list: () => invoke<PluginPackage[]>('plugin_packages_list'),
  import: (source: string, subdirectory?: string) => invoke<PluginPackage>('plugin_packages_import', { source, subdirectory: subdirectory || null }),
  setEnabled: (id: string, enabled: boolean) => invoke<PluginPackage>('plugin_packages_set_enabled', { id, enabled }),
  remove: (id: string) => invoke<void>('plugin_packages_remove', { id }),
  getHooks: () => invoke<unknown>('workflow_hooks_get'),
  saveHooks: (config: unknown) => invoke<void>('workflow_hooks_save', { config }),
}
