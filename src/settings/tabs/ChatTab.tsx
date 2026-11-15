import { FolderIcon as FolderOpen } from '../../components/FileIcon'
import { RefreshCw } from 'lucide-react'
import { open } from '@tauri-apps/plugin-dialog'
import { homeDir, join } from '@tauri-apps/api/path'
import { Select, Input, Toggle } from '../public/controls'
import { SettingRow, SettingsGroup } from '../components'
import { ModelPairSelect } from '../ModelPairSelect'
import { Button } from '../../components/Button'
import { PromptField } from '../ScreenshotTranslationSettings'
import type { I18n, Lang } from '../../components/i18n'
import type { SettingsTab } from '../SettingsShell'
import { AvatarField } from './AvatarField'
import { MemoryIcon, AgentIcon, ProvidersIcon } from '../NavIcons'
import type {
  Settings as SettingsData,
  ChatToolsConfig,
  ChatModeConfig,
} from '../../api/tauri'

/** 对齐 Pi：自定义模型未填 maxTokens 时的缺省，也是唯一的协议兜底。 */
const CHAT_FALLBACK_MAX_OUTPUT_TOKENS = 16384

function formatTokenCount(tokens?: number): string {
  if (!tokens || !Number.isFinite(tokens)) return ''
  return `${tokens.toLocaleString()} tokens`
}

function resolveChatMode(config?: ChatModeConfig | null): Required<ChatModeConfig> {
  return {
    systemPrompt: config?.systemPrompt ?? '',
    webSearch: config?.webSearch ?? true,
    webFetch: config?.webFetch ?? true,
    knowledgeSearch: config?.knowledgeSearch ?? true,
    memoryTools: config?.memoryTools ?? true,
    mcpReadOnly: config?.mcpReadOnly ?? true,
  }
}

interface ChatTabProps {
  settings: SettingsData
  t: I18n
  lang: Lang
  chatConfig: NonNullable<SettingsData['chat']>
  chatTools: ChatToolsConfig
  /** Built-in Agent system prompt (exact string used when systemPrompt is empty). */
  chatDefaults: string | undefined
  /** Built-in Chat runtime prompt (exact string used when chatMode.systemPrompt is empty). */
  chatRuntimeDefaults: string | undefined
  effectiveChatMaxOutput: { maxOutput: number; source: string }
  chatMaxOutputSourceLabel: string
  chatMaxOutputModelLabel: string
  onUpdateChat: (updates: Partial<NonNullable<SettingsData['chat']>>) => void
  onUpdateVoice: (updates: Partial<NonNullable<SettingsData['voice']>>) => void
  onUpdateNativeTools: (updates: Partial<NonNullable<ChatToolsConfig['nativeTools']>>) => void
  onNavigateTab: (tab: SettingsTab) => void
  onUpdateChatTools: (updates: Partial<ChatToolsConfig>) => void
}

/** AI 客户端（聊天）标签页：共用资料 + Kivio Agent / Kivio Chat 两套设置。 */
export function ChatTab({
  settings,
  t,
  lang,
  chatConfig,
  chatTools,
  chatDefaults,
  chatRuntimeDefaults,
  effectiveChatMaxOutput,
  chatMaxOutputSourceLabel,
  chatMaxOutputModelLabel,
  onUpdateChat,
  onUpdateVoice,
  onUpdateNativeTools,
  onUpdateChatTools,
  onNavigateTab,
}: ChatTabProps) {
  const chatMode = resolveChatMode(chatConfig.chatMode)

  const updateChatMode = (updates: Partial<ChatModeConfig>) => {
    onUpdateChat({
      chatMode: {
        ...chatMode,
        ...updates,
      },
    })
  }


  return (
    <>
      <SettingsGroup title={lang === 'zh' ? '个人资料' : 'Profile'}>
        <div className="kv-row">
          <AvatarField
            value={chatConfig.userAvatar || ''}
            onChange={(userAvatar) => onUpdateChat({ userAvatar })}
            zh={lang === 'zh'}
          />
          <div className="kv-row-text">
            <span className="kv-row-label">{lang === 'zh' ? '头像与名称' : 'Avatar and name'}</span>
            <p className="kv-row-desc">{lang === 'zh' ? '显示在你发送的消息旁；点击头像更换。' : 'Shown next to your messages. Click the avatar to change it.'}</p>
          </div>
          <Input
            className="kv-ctl shrink-0"
            value={chatConfig.userDisplayName || ''}
            onChange={(userDisplayName) => onUpdateChat({ userDisplayName })}
            placeholder={lang === 'zh' ? '用户名（选填）' : 'Display name (optional)'}
          />
        </div>
      </SettingsGroup>

      <SettingsGroup title={t.voiceTitle}>
        <SettingRow label={t.voiceChannel}>
          <Select
            className="kv-ctl"
            ariaLabel={t.voiceChannel}
            value={settings.voice?.channel ?? 'auto'}
            onChange={(channel) => onUpdateVoice({ channel: channel === 'auto' ? null : channel as 'codex' | 'live' })}
            options={[
              { value: 'auto', label: t.voiceChannelAuto },
              { value: 'codex', label: t.voiceChannelCodex },
              { value: 'live', label: t.voiceChannelLive },
            ]}
          />
        </SettingRow>
        <SettingRow label={t.voiceLiveProvider}>
          <Select
            className="kv-ctl"
            ariaLabel={t.voiceLiveProvider}
            value={settings.voice?.liveProviderId ?? ''}
            onChange={(liveProviderId) => onUpdateVoice({ liveProviderId: liveProviderId || null })}
            options={[
              { value: '', label: t.voiceAutoProvider },
              ...settings.providers
                .filter((provider) => provider.id)
                .map((provider) => ({ value: provider.id, label: provider.name })),
            ]}
          />
        </SettingRow>
        <SettingRow label={t.voiceOrchestratorModel}>
          <ModelPairSelect
            className="kv-ctl"
            ariaLabel={t.voiceOrchestratorModel}
            providerId={settings.voice?.orchestratorProviderId ?? ''}
            model={settings.voice?.orchestratorModel ?? ''}
            providers={settings.providers}
            inheritLabel={t.voiceUseCurrentModel}
            onChange={(providerId, model) => onUpdateVoice({
              orchestratorProviderId: providerId || null,
              orchestratorModel: model || null,
            })}
          />
        </SettingRow>
      </SettingsGroup>

      {/* ─── Kivio Agent ─── */}
      <SettingsGroup title={t.kivioAgentSection} description={t.kivioAgentSectionHint || undefined}>
        <SettingRow
          label={lang === 'zh' ? '普通对话工作目录' : 'Conversation workspace'}
          description={lang === 'zh'
            ? '新建的非项目 Agent 对话共用此目录，内置与外部 Agent 均适用；已有独立目录继续保留。'
            : 'New non-project chats share this directory across built-in and external agents. Existing separate directories are preserved.'}
          stack
        >
            <div className="kv-path-row">
              <Input
                className="min-w-0 flex-1"
                value={chatTools.nativeTools?.workingDirectory ?? ''}
                placeholder={lang === 'zh' ? '默认：~/Kivio/workspace' : 'Default: ~/Kivio/workspace'}
                onChange={(workingDirectory) => onUpdateNativeTools({ workingDirectory })}
              />
              <Button
                className="shrink-0"
                onClick={async () => {
                  const selected = await open({ directory: true, multiple: false })
                  if (!selected || typeof selected !== 'string') return
                  onUpdateNativeTools({ workingDirectory: selected })
                }}
                data-tauri-drag-region="false"
              >
                <FolderOpen open size={11} />
                {lang === 'zh' ? '选择' : 'Choose'}
              </Button>
              <Button
                className="shrink-0"
                onClick={async () => {
                  const defaultPath = await join(await homeDir(), 'Kivio', 'workspace')
                  onUpdateNativeTools({ workingDirectory: defaultPath })
                }}
                data-tauri-drag-region="false"
              >
                <RefreshCw size={11} />
                {lang === 'zh' ? '恢复默认' : 'Reset'}
              </Button>
            </div>
        </SettingRow>

        <SettingRow
          label={t.chatMaxOutputTokens}
          description={`${lang === 'zh' ? '聊天所选模型：' : 'Chat model: '}${chatMaxOutputModelLabel} · ${lang === 'zh' ? '兜底' : 'fallback'} ${formatTokenCount(CHAT_FALLBACK_MAX_OUTPUT_TOKENS)}`}
        >
          <span className={`kv-tag ${effectiveChatMaxOutput.source === 'fallback' ? 'warn' : 'ok'}`}>
            {chatMaxOutputSourceLabel}
          </span>
          <span className="text-[13px] font-medium tabular-nums text-[var(--text)]">
            {formatTokenCount(effectiveChatMaxOutput.maxOutput)}
          </span>
        </SettingRow>

        <SettingRow label={t.chatDefaultLanguage}>
          <Select
            className="kv-ctl"
            value={chatConfig.defaultLanguage || ''}
            onChange={(defaultLanguage) => onUpdateChat({ defaultLanguage })}
            options={[
              { value: '', label: t.lensLanguageInherit },
              { value: 'zh', label: '中文' },
              { value: 'zh-Hant', label: '繁體中文' },
              { value: 'en', label: 'English' },
            ]}
          />
        </SettingRow>

        <PromptField
          label={t.kivioChatAgentSystemPrompt}
          description={t.chatSystemPromptHint}
          value={chatConfig.systemPrompt || ''}
          defaultText={chatDefaults || ''}
          t={t}
          onChange={(systemPrompt) => onUpdateChat({ systemPrompt })}
        />
      </SettingsGroup>

      <SettingsGroup
        title={t.chatToolsSection}
        actions={
          <>
            <Button size="sm" variant="ghost" onClick={() => onNavigateTab('memory')} data-tauri-drag-region="false">
              <MemoryIcon size={11} />
              {t.tabMemory}
            </Button>
            <Button size="sm" variant="ghost" onClick={() => onNavigateTab('externalAgents')} data-tauri-drag-region="false">
              <AgentIcon size={11} />
              {t.chatOpenExternalAgents}
            </Button>
            <Button size="sm" variant="ghost" onClick={() => onNavigateTab('providers')} data-tauri-drag-region="false">
              <ProvidersIcon size={11} />
              {t.chatOpenProviders}
            </Button>
          </>
        }
      >
        <SettingRow label={t.toolsOnDemand} description={t.toolsOnDemandHint}>
          <Toggle
            ariaLabel={t.toolsOnDemand}
            checked={chatTools.toolsOnDemand ?? true}
            onChange={(toolsOnDemand) => onUpdateChatTools({ toolsOnDemand })}
          />
        </SettingRow>
      </SettingsGroup>

      {/* ─── Kivio Chat ─── */}
      <SettingsGroup title={t.kivioChatSection} description={t.kivioChatSectionHint || undefined}>
        <SettingRow
          label={t.kivioChatWebSearch}
          description={t.kivioChatWebSearchHint}
        >
          <Toggle
            checked={Boolean(chatMode.webSearch)}
            onChange={(webSearch) => updateChatMode({ webSearch })}
          />
        </SettingRow>
        <SettingRow
          label={t.kivioChatWebFetch}
          description={t.kivioChatWebFetchHint}
        >
          <Toggle
            checked={Boolean(chatMode.webFetch)}
            onChange={(webFetch) => updateChatMode({ webFetch })}
          />
        </SettingRow>
        <SettingRow
          label={t.kivioChatKnowledge}
          description={t.kivioChatKnowledgeHint}
        >
          <Toggle
            checked={Boolean(chatMode.knowledgeSearch)}
            onChange={(knowledgeSearch) => updateChatMode({ knowledgeSearch })}
          />
        </SettingRow>
        <SettingRow
          label={t.kivioChatMemory}
          description={t.kivioChatMemoryHint}
        >
          <Toggle
            checked={Boolean(chatMode.memoryTools)}
            onChange={(memoryTools) => updateChatMode({ memoryTools })}
          />
        </SettingRow>
        <SettingRow
          label={t.kivioChatMcpReadonly}
          description={t.kivioChatMcpReadonlyHint}
        >
          <Toggle
            checked={Boolean(chatMode.mcpReadOnly)}
            onChange={(mcpReadOnly) => updateChatMode({ mcpReadOnly })}
          />
        </SettingRow>
        <PromptField
          label={t.kivioChatSystemPrompt}
          description={t.kivioChatSystemPromptHint}
          value={chatMode.systemPrompt || ''}
          defaultText={chatRuntimeDefaults || ''}
          t={t}
          onChange={(systemPrompt) => updateChatMode({ systemPrompt })}
        />
        <p className="kv-group-note">
          {lang === 'zh'
            ? '以上开关仅影响 Kivio Chat。写文件 / Shell / Subagent 始终只在 Kivio Agent 中可用。'
            : 'These toggles only affect Kivio Chat. Write / shell / sub-agents stay Agent-only.'}
        </p>
      </SettingsGroup>
    </>
  )
}
