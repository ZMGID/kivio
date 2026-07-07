import { useCallback, useEffect, useState } from 'react'
import { Loader2, X } from 'lucide-react'
import { api, type EmailAccountConfig, type EmailProviderPreset } from '../api/tauri'
import { i18n, type Lang } from './i18n'
import { Input, Select } from './components'
import { Button } from '../components/Button'

const ENCRYPTION_OPTIONS = [
  { value: 'tls', label: 'TLS' },
  { value: 'start-tls', label: 'STARTTLS' },
  { value: 'none', label: 'None' },
]

// eslint-disable-next-line react-refresh/only-export-components -- draft factory shared with Settings; hot-reload loss here is acceptable
export function defaultEmailDraft(preset?: EmailProviderPreset): EmailAccountConfig {
  return {
    id: '',
    email: '',
    displayName: '',
    password: '',
    imapHost: preset?.imapHost ?? 'imap.gmail.com',
    imapPort: preset?.imapPort ?? 993,
    imapEncryption: preset?.imapEncryption ?? 'tls',
    smtpHost: preset?.smtpHost ?? 'smtp.gmail.com',
    smtpPort: preset?.smtpPort ?? 587,
    smtpEncryption: preset?.smtpEncryption ?? 'start-tls',
    isDefault: true,
  }
}

type Props = {
  lang: Lang
  initial?: EmailAccountConfig
  existingAccounts?: EmailAccountConfig[]
  onSave: (account: EmailAccountConfig) => void
  onCancel: () => void
}

export function EmailConnectorForm({ lang, initial, existingAccounts = [], onSave, onCancel }: Props) {
  const t = i18n[lang]
  const [presets, setPresets] = useState<EmailProviderPreset[]>([])
  const [presetId, setPresetId] = useState('gmail')
  const [draft, setDraft] = useState<EmailAccountConfig>(() => initial ?? defaultEmailDraft())
  const [testing, setTesting] = useState(false)
  const [testMessage, setTestMessage] = useState<{ ok: boolean; text: string } | null>(null)

  useEffect(() => {
    void api.listEmailProviderPresets().then(setPresets).catch(() => setPresets([]))
  }, [])

  const applyPreset = useCallback((id: string, list: EmailProviderPreset[]) => {
    const preset = list.find((item) => item.id === id)
    if (!preset || preset.id === 'custom') return
    setDraft((prev) => ({
      ...prev,
      imapHost: preset.imapHost,
      imapPort: preset.imapPort,
      imapEncryption: preset.imapEncryption,
      smtpHost: preset.smtpHost,
      smtpPort: preset.smtpPort,
      smtpEncryption: preset.smtpEncryption,
    }))
  }, [])

  const patch = (updates: Partial<EmailAccountConfig>) => {
    setDraft((prev) => ({ ...prev, ...updates }))
    setTestMessage(null)
  }

  const canSave =
    draft.email.trim().length > 0 &&
    draft.password.trim().length > 0 &&
    draft.imapHost.trim().length > 0 &&
    draft.smtpHost.trim().length > 0

  const runTest = async () => {
    if (!canSave) return
    setTesting(true)
    setTestMessage(null)
    try {
      const output = await api.testHimalayaEmail(draft, existingAccounts)
      setTestMessage({ ok: true, text: output || t.connectorsEmailTestOk })
    } catch (err) {
      setTestMessage({ ok: false, text: String(err) })
    } finally {
      setTesting(false)
    }
  }

  return (
    <div className="mt-2 space-y-2" onClick={(e) => e.stopPropagation()}>
      <div className="kv-row-desc text-[12px] opacity-80">{t.connectorsEmailHint}</div>
      {presets.length > 0 && (
        <Select
          value={presetId}
          onChange={(id) => {
            setPresetId(id)
            applyPreset(id, presets)
          }}
          options={presets.map((preset) => ({ value: preset.id, label: preset.label }))}
        />
      )}
      <Input
        value={draft.email}
        onChange={(v) => patch({ email: v })}
        placeholder={t.connectorsEmailAddress}
      />
      <Input
        value={draft.displayName}
        onChange={(v) => patch({ displayName: v })}
        placeholder={t.connectorsEmailDisplayName}
      />
      <Input
        value={draft.password}
        onChange={(v) => patch({ password: v })}
        type="password"
        mono
        placeholder={t.connectorsEmailPassword}
      />
      <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
        <Input
          value={draft.imapHost}
          onChange={(v) => patch({ imapHost: v })}
          mono
          placeholder={t.connectorsEmailImapHost}
        />
        <Input
          value={String(draft.imapPort)}
          onChange={(v) => patch({ imapPort: Number(v) || 993 })}
          mono
          placeholder={t.connectorsEmailImapPort}
        />
        <Select
          value={draft.imapEncryption}
          onChange={(v) => patch({ imapEncryption: v })}
          options={ENCRYPTION_OPTIONS}
        />
        <div />
        <Input
          value={draft.smtpHost}
          onChange={(v) => patch({ smtpHost: v })}
          mono
          placeholder={t.connectorsEmailSmtpHost}
        />
        <Input
          value={String(draft.smtpPort)}
          onChange={(v) => patch({ smtpPort: Number(v) || 587 })}
          mono
          placeholder={t.connectorsEmailSmtpPort}
        />
        <Select
          value={draft.smtpEncryption}
          onChange={(v) => patch({ smtpEncryption: v })}
          options={ENCRYPTION_OPTIONS}
        />
      </div>
      {testMessage && (
        <div
          className={`kv-row-desc text-[12px] ${testMessage.ok ? 'text-emerald-600 dark:text-emerald-400' : 'text-red-500 dark:text-red-400'}`}
        >
          {testMessage.ok ? t.connectorsEmailTestOk : t.connectorsEmailTestFail}: {testMessage.text}
        </div>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          disabled={!canSave || testing}
          onClick={() => void runTest()}
          data-tauri-drag-region="false"
        >
          {testing ? <Loader2 size={10} className="animate-spin" /> : null}
          {t.connectorsEmailTest}
        </Button>
        <Button
          variant="primary"
          size="sm"
          disabled={!canSave}
          onClick={() => onSave(draft)}
          data-tauri-drag-region="false"
        >
          {t.connectorsEmailSave}
        </Button>
        <Button
          size="sm"
          onClick={onCancel}
          data-tauri-drag-region="false"
        >
          <X size={10} />
        </Button>
      </div>
    </div>
  )
}
