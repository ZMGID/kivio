import { useState } from 'react'
import { RefreshCw } from 'lucide-react'
import { Toggle, Select, Input, SettingRow, SettingsGroup, PermissionItem } from '../components'
import { Button } from '../../components/Button'
import { UI_FONT_PX_MIN, UI_FONT_PX_MAX } from '../uiFont'
import type { I18n, Lang } from '../../components/i18n'
import type { Settings as SettingsData, PermissionStatus } from '../../api/tauri'

/**
 * 可搜索字体选择器：聚焦展开、输入过滤，每项以自身字体预览。界面字体/代码字体共用。
 * 原在 SettingsShell 模块作用域，只被外观设置使用，随之搬来。
 */
function FontPicker({ value, systemFonts, placeholder, defaultLabel, emptyText, onChange }: {
  value: string
  systemFonts: string[]
  placeholder: string
  defaultLabel: string
  emptyText: string
  onChange: (v: string) => void
}) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const q = query.trim().toLowerCase()
  const filtered = (q ? systemFonts.filter((f) => f.toLowerCase().includes(q)) : systemFonts).slice(0, 100)
  const select = (name: string) => {
    onChange(name)
    setOpen(false)
    setQuery('')
  }
  return (
    <div className="relative w-56">
      <Input
        value={open ? query : (value || defaultLabel)}
        onChange={(v) => { setQuery(v); if (!open) setOpen(true) }}
        onFocus={() => { setQuery(''); setOpen(true) }}
        onBlur={() => window.setTimeout(() => setOpen(false), 120)}
        placeholder={placeholder}
      />
      {open && (
        <div className="absolute right-0 z-50 mt-1 max-h-64 w-full overflow-auto kv-menu">
          <button
            type="button"
            className={`kv-menu-row truncate hover:bg-[var(--theme-surface-hover)] ${value === '' ? 'font-semibold text-[var(--text)]' : 'text-[var(--text-muted)]'}`}
            onMouseDown={(e) => { e.preventDefault(); select('') }}
          >
            {defaultLabel}
          </button>
          {filtered.map((f) => (
            <button
              key={f}
              type="button"
              className={`kv-menu-row truncate hover:bg-[var(--theme-surface-hover)] ${value === f ? 'bg-[var(--theme-surface-active)] font-semibold text-[var(--text)]' : 'text-[var(--text-muted)]'}`}
              style={{ fontFamily: `"${f}"` }}
              onMouseDown={(e) => { e.preventDefault(); select(f) }}
            >
              {f}
            </button>
          ))}
          {filtered.length === 0 && (
            <div className="px-3 py-2 text-[12px] text-neutral-400">{emptyText}</div>
          )}
        </div>
      )}
    </div>
  )
}

function AppearanceSubsection({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="settings-appearance-subsection">
      <div className="settings-appearance-subsection-title">{title}</div>
      <div className="settings-appearance-subsection-body">{children}</div>
    </div>
  )
}

/** 通用外观：语言、界面字号、界面字体、代码字体。主题与材质由独立主题页管理。 */
export function AppearanceGroup({
  settings,
  t,
  lang,
  systemFonts,
  uiFontPxInput,
  onUpdateSettings,
  onUiFontPxInputChange,
  onCommitUiFontPx,
}: {
  settings: SettingsData
  t: I18n
  lang: Lang
  systemFonts: string[]
  uiFontPxInput: string
  onUpdateSettings: (updates: Partial<SettingsData>) => void
  onUiFontPxInputChange: (value: string) => void
  onCommitUiFontPx: (value: string, commit: boolean) => void
}) {
  return (
    <SettingsGroup title={lang === 'zh' ? '外观' : 'Appearance'} className="settings-appearance-group">
      <AppearanceSubsection title={lang === 'zh' ? '界面' : 'Interface'}>
        <SettingRow label={t.language}>
          <Select
            className="w-36"
            value={settings.settingsLanguage || 'zh'}
            onChange={(v) => onUpdateSettings({ settingsLanguage: v as 'zh' | 'en' })}
            options={[
              { value: 'zh', label: '中文' },
              { value: 'en', label: 'English' },
            ]}
          />
        </SettingRow>
      </AppearanceSubsection>


      <AppearanceSubsection title={lang === 'zh' ? '字体' : 'Typography'}>
        <SettingRow
          label={lang === 'zh' ? '界面字号' : 'UI size'}
        >
          <div className="flex items-center gap-2">
            <Input
              type="number"
              value={uiFontPxInput}
              onChange={(v) => { onUiFontPxInputChange(v); onCommitUiFontPx(v, false) }}
              onBlur={() => onCommitUiFontPx(uiFontPxInput, true)}
              min={UI_FONT_PX_MIN}
              max={UI_FONT_PX_MAX}
              className="!w-16 text-center"
            />
            <span className="text-[13px] text-[var(--text-faint)]">px</span>
          </div>
        </SettingRow>
        <SettingRow
          label={lang === 'zh' ? '界面字体' : 'UI font'}
        >
          <FontPicker
            value={settings.uiFontFamily ?? ''}
            systemFonts={systemFonts}
            placeholder={lang === 'zh' ? '搜索字体…' : 'Search fonts…'}
            defaultLabel={lang === 'zh' ? '系统默认' : 'System default'}
            emptyText={lang === 'zh' ? '无匹配字体' : 'No matching fonts'}
            onChange={(v) => onUpdateSettings({ uiFontFamily: v })}
          />
        </SettingRow>
        <SettingRow
          label={lang === 'zh' ? '代码字体' : 'Code font'}
        >
          <FontPicker
            value={settings.uiFontMono ?? ''}
            systemFonts={systemFonts}
            placeholder={lang === 'zh' ? '搜索字体…' : 'Search fonts…'}
            defaultLabel={lang === 'zh' ? '系统默认' : 'System default'}
            emptyText={lang === 'zh' ? '无匹配字体' : 'No matching fonts'}
            onChange={(v) => onUpdateSettings({ uiFontMono: v })}
          />
        </SettingRow>
      </AppearanceSubsection>
    </SettingsGroup>
  )
}

/** 行为：开机自启 / 启动后最小化到托盘 / 关闭时保持聊天窗口 / 回复完成通知 / 失败重试。 */
export function BehaviorGroup({
  settings,
  t,
  lang,
  retryAttemptsInput,
  onUpdateSettings,
  onRetryAttemptsChange,
  onRetryAttemptsBlur,
}: {
  settings: SettingsData
  t: I18n
  lang: Lang
  retryAttemptsInput: string
  onUpdateSettings: (updates: Partial<SettingsData>) => void
  onRetryAttemptsChange: (value: string) => void
  onRetryAttemptsBlur: () => void
}) {
  return (
    <SettingsGroup title={lang === 'zh' ? '行为' : 'Behavior'}>
      <SettingRow label={t.launchAtStartup}>
        <Toggle
          checked={settings.launchAtStartup}
          onChange={(v) => onUpdateSettings({ launchAtStartup: v })}
        />
      </SettingRow>
      <SettingRow label={t.launchMinimizedToTray} description={t.launchMinimizedToTrayDesc}>
        <Toggle
          checked={settings.launchMinimizedToTray}
          onChange={(v) => onUpdateSettings({ launchMinimizedToTray: v })}
        />
      </SettingRow>
      <SettingRow label={t.keepChatWindowAlive} description={t.keepChatWindowAliveDesc}>
        <Toggle
          checked={settings.keepChatWindowAlive ?? false}
          onChange={(v) => onUpdateSettings({ keepChatWindowAlive: v })}
          ariaLabel={t.keepChatWindowAlive}
        />
      </SettingRow>
      <SettingRow label={t.chatCompletionNotifications} description={t.chatCompletionNotificationsDesc}>
        <Toggle
          checked={settings.chatCompletionNotifications}
          onChange={(v) => onUpdateSettings({ chatCompletionNotifications: v })}
          ariaLabel={t.chatCompletionNotifications}
        />
      </SettingRow>
      <SettingRow label={t.retryEnabled}>
        <Toggle
          checked={settings.retryEnabled}
          onChange={(v) => onUpdateSettings({ retryEnabled: v })}
          ariaLabel={t.retryEnabled}
        />
      </SettingRow>
      {settings.retryEnabled && (
        <SettingRow label={t.retryAttempts}>
          <Input
            type="number"
            value={retryAttemptsInput}
            onChange={onRetryAttemptsChange}
            onBlur={onRetryAttemptsBlur}
            placeholder="5"
            min={1}
            max={8}
            className="!w-20 text-center"
          />
        </SettingRow>
      )}
    </SettingsGroup>
  )
}

/** macOS 权限：辅助功能 / 屏幕录制。仅 macOS 渲染，由 shell 判定后调用。 */
export function PermissionsGroup({
  t,
  permissionStatus,
  permissionsLoading,
  onOpenPermissionSettings,
  onRefreshPermissions,
}: {
  t: I18n
  permissionStatus: PermissionStatus
  permissionsLoading: boolean
  onOpenPermissionSettings: (target: 'accessibility' | 'screen-recording') => void
  onRefreshPermissions: () => void
}) {
  return (
    <SettingsGroup title={t.permissions}>
      <PermissionItem
        label={t.accessibilityPermission}
        granted={permissionStatus.accessibility}
        grantedText={t.permissionGranted}
        missingText={t.permissionMissing}
        actionLabel={t.openSystemSettings}
        onOpen={() => onOpenPermissionSettings('accessibility')}
      />
      <PermissionItem
        label={t.screenRecordingPermission}
        granted={permissionStatus.screenRecording}
        grantedText={t.permissionGranted}
        missingText={t.permissionMissing}
        actionLabel={t.openSystemSettings}
        onOpen={() => onOpenPermissionSettings('screen-recording')}
      />
      <div className="flex justify-end py-2">
        <Button
          size="sm"
          onClick={onRefreshPermissions}
          disabled={permissionsLoading}
          data-tauri-drag-region="false"
        >
          <RefreshCw size={10} className={permissionsLoading ? 'animate-spin' : ''} />
          {t.refreshPermissions}
        </Button>
      </div>
    </SettingsGroup>
  )
}
