import { invoke } from '@tauri-apps/api/core'
import { isTauriRuntime } from './tauri'

export interface NativeProviderSummary {
  id: string
  name: string
  baseUrl?: string | null
  api?: string | null
  modelCount: number
  isDefault: boolean
}

export interface DetectedExternalAgent {
  id: string
  name: string
  available: boolean
  nativeProviders?: NativeProviderSummary[]
  path?: string | null
  version?: string | null
  models: Array<{ id: string; label: string; contextWindowTokens?: number | null; context_window_tokens?: number | null }>
  reasoningOptions?: Array<{ id: string; label: string }>
  reasoning_options?: Array<{ id: string; label: string }>
  sandboxOptions?: Array<{ id: string; label: string }>
  sandbox_options?: Array<{ id: string; label: string }>
  authStatus?: string | null
  auth_status?: string | null
  disabled?: boolean
  supportsSteering?: boolean
  supports_steering?: boolean
  supportsFollowUp?: boolean
  supports_follow_up?: boolean
}

/** `chat_external_cli_scan_cc_switch` response; secret keys are never returned. */
export interface CcSwitchProvider {
  agentId: string
  id: string
  name: string
  remark: string
  env: Array<{ key: string; value: string }>
  configToml: string
  authJson: string
  hasApiKey: boolean
  isCurrent: boolean
}

export interface CcSwitchScan {
  providers: CcSwitchProvider[]
  skipped: number
}

export interface DshPluginSettingsSnapshot {
  settingsPath: string
  shell: {
    timeoutMs: number | null
    maxOutputBytes: number | null
    timeoutMsDefault: number
    maxOutputBytesDefault: number
  }
  agentLoop: {
    maxParallelToolCalls: number | null
    maxParallelToolCallsDefault: number
  }
  webSearch: {
    baseUrl: string | null
    maxUses: number | null
    apiKeyEnv: string
    apiKeyConfigured: boolean
    apiKeyWritable: boolean
    baseUrlDefault: string
    maxUsesDefault: number
  }
}

export interface DshPluginSettingsPatch {
  shell?: {
    timeoutMs?: number | null
    maxOutputBytes?: number | null
  }
  agentLoop?: {
    maxParallelToolCalls?: number | null
  }
  webSearch?: {
    baseUrl?: string | null
    maxUses?: number | null
    apiKey?: string
  }
}

export interface DshPluginEntry {
  id: string
  moduleName: string
  enabled: boolean
}

export interface DshOfficialCredential {
  configured: boolean
  writable: boolean
}

export interface DshNativeProviderModel {
  id: string
  name: string
}

export interface DshNativeProviderDetail {
  id: string
  name: string
  baseUrl: string
  api: string
  apiKey: string
  apiKeyEnv: string
  models: DshNativeProviderModel[]
  defaultModel: string
}

export interface DshAgentPresetOption {
  id: string
  label: string
  description?: string | null
}

export interface PiExtensionInventory {
  agentDir: string
  extensionsDir: string
  packages: PiExtensionPackage[]
  localExtensions: PiLocalExtension[]
}

export interface PiExtensionPackage {
  source: string
  name: string
  version: string | null
  description: string | null
  path: string | null
  enabled: boolean
  canToggle: boolean
  hasExtensions: boolean
  extensionEntries: number
  resources: string[]
}

export interface PiLocalExtension {
  relativePath: string
  name: string
  path: string
  enabled: boolean
  kind: 'file' | 'directory'
}

export interface PiExtensionCommandResult {
  output: string
}

export interface PiSkillInventory {
  agentDir: string
  piSkillsDir: string
  agentsSkillsDir: string
  skillCommandsEnabled: boolean
  configuredPaths: PiSkillConfiguredPath[]
  skills: PiSkillEntry[]
}

export interface PiSkillConfiguredPath {
  path: string
  exists: boolean
}

export interface PiSkillEntry {
  name: string
  description: string | null
  path: string
  sourceKind: 'pi' | 'agents' | 'configured' | 'package'
  packageSource: string | null
  packageRoot: string | null
  enabled: boolean
  canToggle: boolean
  canRemove: boolean
}

export interface ExternalCliInstallInfo {
  agentId: string
  localVersion: string | null
  latestVersion: string | null
  updateAvailable: boolean
  command: string | null
  docsUrl: string
  configDir: string | null
}

/** Settings and Chat share one transport owner for external CLI configuration. */
export const externalCliSettingsApi = {
  async detectExternalAgents(forceRefresh = false, conversationId?: string | null): Promise<DetectedExternalAgent[]> {
    if (!isTauriRuntime()) {
      return [{ id: 'claude', name: 'Claude Code', available: false, models: [{ id: 'default', label: 'Default' }] }]
    }
    const result = await invoke<{ success: boolean; agents: DetectedExternalAgent[] }>(
      'chat_detect_external_agents',
      { forceRefresh, conversationId },
    )
    return result.agents ?? []
  },

  async detectExternalAgentModels(agentId: string, conversationId?: string | null, force = false): Promise<{
    models: DetectedExternalAgent['models']
    reasoningOptions: NonNullable<DetectedExternalAgent['reasoningOptions']>
    reasoningByModel: Record<string, NonNullable<DetectedExternalAgent['reasoningOptions']>>
    source: 'probed' | 'fallback'
    probeError?: string
    currentModel?: string | null
    currentReasoning?: string | null
  }> {
    if (!isTauriRuntime()) {
      return { models: [], reasoningOptions: [], reasoningByModel: {}, source: 'probed' }
    }
    const result = await invoke<{
      success: boolean
      models?: DetectedExternalAgent['models']
      reasoningOptions?: NonNullable<DetectedExternalAgent['reasoningOptions']>
      reasoningByModel?: Record<string, NonNullable<DetectedExternalAgent['reasoningOptions']>>
      source?: 'probed' | 'fallback'
      probeError?: string
      currentModel?: string | null
      currentReasoning?: string | null
    }>('chat_detect_external_agent_models', { agentId, conversationId, force })
    return {
      models: result.models ?? [],
      reasoningOptions: result.reasoningOptions ?? [],
      reasoningByModel: result.reasoningByModel ?? {},
      source: result.source ?? 'probed',
      probeError: result.probeError,
      currentModel: result.currentModel ?? null,
      currentReasoning: result.currentReasoning ?? null,
    }
  },

  async externalCliInstallInfo(agentId: string): Promise<ExternalCliInstallInfo | null> {
    if (!isTauriRuntime()) return null
    return invoke<ExternalCliInstallInfo>('chat_external_cli_install_info', { agentId })
  },

  async externalCliInstall(agentId: string): Promise<void> {
    if (!isTauriRuntime()) return
    await invoke('chat_external_cli_install', { agentId })
  },

  async piExtensionsInventory(): Promise<PiExtensionInventory | null> {
    if (!isTauriRuntime()) return null
    return invoke<PiExtensionInventory>('chat_pi_extensions_inventory')
  },

  async piExtensionSetEnabled(kind: 'package' | 'local', id: string, enabled: boolean): Promise<void> {
    if (!isTauriRuntime()) return
    await invoke('chat_pi_extension_set_enabled', { kind, id, enabled })
  },

  async piExtensionInstall(source: string): Promise<PiExtensionCommandResult> {
    return invoke<PiExtensionCommandResult>('chat_pi_extension_install', { source })
  },

  async piExtensionUpdate(source?: string): Promise<PiExtensionCommandResult> {
    return invoke<PiExtensionCommandResult>('chat_pi_extension_update', { source: source ?? null })
  },

  async piExtensionRemove(source: string): Promise<PiExtensionCommandResult> {
    return invoke<PiExtensionCommandResult>('chat_pi_extension_remove', { source })
  },

  async piExtensionOpen(kind: 'package' | 'local', id: string): Promise<void> {
    if (!isTauriRuntime()) return
    await invoke('chat_pi_extension_open', { kind, id })
  },

  async piExtensionsOpenDir(): Promise<void> {
    if (!isTauriRuntime()) return
    await invoke('chat_pi_extensions_open_dir')
  },

  async piSkillsInventory(): Promise<PiSkillInventory | null> {
    if (!isTauriRuntime()) return null
    return invoke<PiSkillInventory>('chat_pi_skills_inventory')
  },

  async piSkillSetEnabled(skill: PiSkillEntry, enabled: boolean): Promise<void> {
    if (!isTauriRuntime()) return
    await invoke('chat_pi_skill_set_enabled', { path: skill.path, packageSource: skill.packageSource, enabled })
  },

  async piSkillCommandsSetEnabled(enabled: boolean): Promise<void> {
    if (!isTauriRuntime()) return
    await invoke('chat_pi_skill_commands_set_enabled', { enabled })
  },

  async piSkillAddPath(path: string): Promise<void> {
    if (!isTauriRuntime()) return
    await invoke('chat_pi_skill_add_path', { path })
  },

  async piSkillRemovePath(path: string): Promise<void> {
    if (!isTauriRuntime()) return
    await invoke('chat_pi_skill_remove_path', { path })
  },

  async piSkillRemove(skill: PiSkillEntry): Promise<void> {
    if (!isTauriRuntime()) return
    await invoke('chat_pi_skill_remove', { path: skill.path, packageSource: skill.packageSource })
  },

  async piSkillOpen(skill: PiSkillEntry): Promise<void> {
    if (!isTauriRuntime()) return
    await invoke('chat_pi_skill_open', { path: skill.path, packageSource: skill.packageSource })
  },

  async piSkillsOpenDir(kind: 'pi' | 'agents'): Promise<void> {
    if (!isTauriRuntime()) return
    await invoke('chat_pi_skills_open_dir', { kind })
  },

  async dshPluginSettingsGet(): Promise<DshPluginSettingsSnapshot | null> {
    if (!isTauriRuntime()) return null
    return invoke<DshPluginSettingsSnapshot>('chat_dsh_plugin_settings_get')
  },

  async dshPluginSettingsSave(patch: DshPluginSettingsPatch): Promise<DshPluginSettingsSnapshot> {
    return invoke<DshPluginSettingsSnapshot>('chat_dsh_plugin_settings_save', { patch })
  },

  async dshPluginInventory(): Promise<DshPluginEntry[]> {
    if (!isTauriRuntime()) return []
    return invoke<DshPluginEntry[]>('chat_dsh_plugin_inventory')
  },

  async dshOpenSettingsFile(): Promise<void> {
    if (!isTauriRuntime()) return
    await invoke('chat_dsh_open_settings_file')
  },

  async dshOfficialCredentialStatus(): Promise<DshOfficialCredential> {
    if (!isTauriRuntime()) return { configured: false, writable: true }
    return invoke<DshOfficialCredential>('chat_dsh_official_credential_status')
  },

  async dshOfficialCredentialSave(apiKey: string): Promise<DshOfficialCredential> {
    return invoke<DshOfficialCredential>('chat_dsh_official_credential_save', { apiKey })
  },

  async dshNativeProviderGet(id: string): Promise<DshNativeProviderDetail> {
    return invoke<DshNativeProviderDetail>('chat_dsh_native_provider_get', { id })
  },

  async dshNativeProviderDelete(id: string): Promise<void> {
    if (!isTauriRuntime()) return
    await invoke('chat_dsh_native_provider_delete', { id })
  },

  async listDshAgentPresets(): Promise<DshAgentPresetOption[]> {
    if (!isTauriRuntime()) return []
    const list = await invoke<DshAgentPresetOption[]>('chat_dsh_list_agent_presets')
    return Array.isArray(list) ? list : []
  },

  async externalCliProviderCleanup(agentId: string, providerId: string, nativeProviderId?: string, providerName?: string): Promise<void> {
    if (!isTauriRuntime()) return
    await invoke('chat_external_cli_provider_cleanup', { agentId, providerId, nativeProviderId, providerName })
  },

  async externalCliFetchRelayModels(baseUrl: string, apiKey: string): Promise<string[]> {
    if (!isTauriRuntime()) return []
    return invoke<string[]>('chat_external_cli_fetch_relay_models', { baseUrl, apiKey })
  },

  async externalCliScanCcSwitch(): Promise<CcSwitchScan> {
    if (!isTauriRuntime()) return { providers: [], skipped: 0 }
    return invoke<CcSwitchScan>('chat_external_cli_scan_cc_switch')
  },
}

export async function onExternalCliInstallLog(
  handler: (event: { agentId: string; line: string | null; done: boolean; success: boolean }) => void,
): Promise<() => void> {
  if (!isTauriRuntime()) return () => {}
  const { listen } = await import('@tauri-apps/api/event')
  return listen<{ agentId: string; line: string | null; done: boolean; success: boolean }>(
    'external-cli-install',
    (event) => handler(event.payload),
  )
}

export async function onExternalAgentsUpdated(
  handler: (agents: DetectedExternalAgent[]) => void,
): Promise<() => void> {
  if (!isTauriRuntime()) return () => {}
  const { listen } = await import('@tauri-apps/api/event')
  return listen<{ agents: DetectedExternalAgent[] }>(
    'external-agents-updated',
    (event) => handler(event.payload.agents ?? []),
  )
}
