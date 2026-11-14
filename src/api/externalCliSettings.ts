import { invoke } from '@tauri-apps/api/core'
import type { ProviderCapabilities } from '../generated/chatProtocol'
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
  models: Array<{ id: string; label: string; contextWindowTokens?: number | null; context_window_tokens?: number | null; capabilities?: { inputModalities?: string[] | null; serviceTiers: Array<{ id: string; label: string }>; defaultServiceTier?: string | null } | null }>
  reasoningOptions?: Array<{ id: string; label: string }>
  reasoning_options?: Array<{ id: string; label: string }>
  sandboxOptions?: Array<{ id: string; label: string }>
  sandbox_options?: Array<{ id: string; label: string }>
  authStatus?: string | null
  auth_status?: string | null
  disabled?: boolean
  capabilities: ProviderCapabilities
}

export interface ExternalAgentModels {
  models: DetectedExternalAgent['models']
  reasoningOptions: NonNullable<DetectedExternalAgent['reasoningOptions']>
  reasoningByModel: Record<string, NonNullable<DetectedExternalAgent['reasoningOptions']>>
  source: 'probed' | 'fallback'
  probeError?: string
  currentModel?: string | null
  currentReasoning?: string | null
  refreshing?: boolean
}

function modelResult(result: Partial<ExternalAgentModels>): ExternalAgentModels {
  return {
    models: result.models ?? [],
    reasoningOptions: result.reasoningOptions ?? [],
    reasoningByModel: result.reasoningByModel ?? {},
    source: result.source ?? 'probed',
    probeError: result.probeError,
    currentModel: result.currentModel ?? null,
    currentReasoning: result.currentReasoning ?? null,
    ...(result.refreshing !== undefined ? { refreshing: result.refreshing } : {}),
  }
}

/** `chat_external_cli_scan_cc_switch` response; secret keys are never returned. */
export interface CcSwitchProvider {
  agentId: string
  id: string
  name: string
  remark: string
  hasApiKey: boolean
  isCurrent: boolean
}

export interface ImportedCcSwitchProvider extends CcSwitchProvider {
  env: Array<{ key: string; value: string }>
  configToml: string
  authJson: string
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
  planId: string | null
  manualReason: string | null
  binaryPath: string | null
  targetVersion: string | null
  environment: string | null
  source: string | null
}

export interface ExternalCliAccount {
  agentId: string
  account: { type: string | null; email: string | null; plan: string | null } | null
  windows: Array<{ name: string; usedPercent: number; resetsAt: string | number | null; windowDurationMins: number | null }>
  credits: { label: string; unlimited?: boolean; balance?: string | null; used?: number | null; limit?: number | null; currency?: string | null } | null
  unavailable: string | null
  checkedAt: number
}

/** Settings and Chat share one transport owner for external CLI configuration. */
export const externalCliSettingsApi = {
  async getExternalCliAccount(agentId: string, conversationId?: string | null): Promise<ExternalCliAccount> {
    if (!isTauriRuntime()) throw new Error('Account discovery requires the desktop app')
    return invoke<ExternalCliAccount>('chat_external_cli_account', { agentId, conversationId })
  },
  async detectExternalAgents(forceRefresh = false, conversationId?: string | null): Promise<DetectedExternalAgent[]> {
    if (!isTauriRuntime()) {
      return [{
        id: 'claude',
        name: 'Claude Code',
        available: false,
        models: [{ id: 'default', label: 'Default' }],
        capabilities: {
          steering: 'unavailable',
          followUp: 'unavailable',
          imageInput: 'unavailable',
          rewind: 'unavailable',
          conversationBranch: 'unavailable',
          importHistory: 'unavailable',
          account: 'unavailable',
        },
      }]
    }
    const result = await invoke<{ success: boolean; agents: DetectedExternalAgent[] }>(
      'chat_detect_external_agents',
      { forceRefresh, conversationId },
    )
    return result.agents ?? []
  },

  async detectExternalAgentModels(agentId: string, conversationId?: string | null, force = false, onSnapshot?: (result: ExternalAgentModels) => void): Promise<ExternalAgentModels> {
    if (!isTauriRuntime()) {
      return { models: [], reasoningOptions: [], reasoningByModel: {}, source: 'probed' }
    }
    if (onSnapshot) {
      const snapshot = modelResult(await invoke<Partial<ExternalAgentModels>>(
        'chat_detect_external_agent_models', { agentId, conversationId, force, snapshotOnly: true },
      ))
      if (!snapshot.refreshing) return snapshot
      onSnapshot(snapshot)
    }
    return modelResult(await invoke<Partial<ExternalAgentModels>>(
      'chat_detect_external_agent_models', { agentId, conversationId, force },
    ))
  },

  async externalCliInstallInfo(agentId: string): Promise<ExternalCliInstallInfo | null> {
    if (!isTauriRuntime()) return null
    return invoke<ExternalCliInstallInfo>('chat_external_cli_install_info', { agentId })
  },

  async externalCliInstall(agentId: string, planId: string): Promise<void> {
    if (!isTauriRuntime()) return
    await invoke('chat_external_cli_install', { agentId, planId })
  },

  /** 在系统文件管理器里打开该 CLI 的配置目录。 */
  async openConfigDir(agentId: string): Promise<void> {
    if (!isTauriRuntime()) return
    await invoke('chat_external_cli_open_config_dir', { agentId })
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

  async externalCliFetchRelayModels(baseUrl: string, apiKey: string): Promise<string[]> {
    if (!isTauriRuntime()) return []
    return invoke<string[]>('chat_external_cli_fetch_relay_models', { baseUrl, apiKey })
  },

  async externalCliScanCcSwitch(): Promise<CcSwitchScan> {
    if (!isTauriRuntime()) return { providers: [], skipped: 0 }
    return invoke<CcSwitchScan>('chat_external_cli_scan_cc_switch')
  },

  async externalCliImportCcSwitch(selections: Array<{ agentId: string; providerId: string }>): Promise<ImportedCcSwitchProvider[]> {
    if (!isTauriRuntime()) return []
    return invoke<ImportedCcSwitchProvider[]>('chat_external_cli_import_cc_switch', { selections })
  },
}

export interface ExternalCliInstallLog {
  agentId: string
  planId: string
  line: string | null
  done: boolean
  success: boolean
}

export async function onExternalCliInstallLog(
  handler: (event: ExternalCliInstallLog) => void,
): Promise<() => void> {
  if (!isTauriRuntime()) return () => {}
  const { listen } = await import('@tauri-apps/api/event')
  return listen<ExternalCliInstallLog>(
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
