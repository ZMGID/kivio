import { useCallback, useEffect, useState } from 'react'
import { Loader2, Plus, Trash2, X } from 'lucide-react'
import {
  api,
  type EmailAccountConfig,
  type HimalayaStatus,
} from '../api/tauri'
import { i18n, type Lang } from './i18n'
import { EmailConnectorForm } from './EmailConnectorForm'
import { EmailBrandIcon } from './ConnectorBrandIcons'
import { Button, IconButton } from '../components/Button'

type Props = {
  lang: Lang
  open: boolean
  accounts: EmailAccountConfig[]
  onAccountsChange: (accounts: EmailAccountConfig[]) => void
  onClose: () => void
}

export function EmailConnectorModal({
  lang,
  open,
  accounts,
  onAccountsChange,
  onClose,
}: Props) {
  const t = i18n[lang]
  const [status, setStatus] = useState<HimalayaStatus | null>(null)
  const [installing, setInstalling] = useState(false)
  const [installError, setInstallError] = useState('')
  const [showAddForm, setShowAddForm] = useState(false)
  const [editingIndex, setEditingIndex] = useState<number | null>(null)

  const refreshStatus = useCallback(async () => {
    try {
      setStatus(await api.himalayaStatus())
    } catch {
      setStatus({ installed: false, version: null, path: null })
    }
  }, [])

  useEffect(() => {
    if (!open) return
    setInstallError('')
    setShowAddForm(accounts.length === 0)
    setEditingIndex(null)
    void refreshStatus()
  }, [accounts.length, open, refreshStatus])

  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, open])

  const handleInstall = async () => {
    setInstalling(true)
    setInstallError('')
    try {
      const result = await api.himalayaInstall()
      if (!result.ok) {
        setInstallError(result.message)
      }
      await refreshStatus()
    } catch (err) {
      setInstallError(String(err))
    } finally {
      setInstalling(false)
    }
  }

  const removeAccount = (index: number) => {
    onAccountsChange(accounts.filter((_, i) => i !== index))
  }

  const saveAccount = (account: EmailAccountConfig) => {
    const next = [...accounts]
    if (editingIndex !== null) {
      next[editingIndex] = account
    } else {
      next.push(account)
    }
    if (next.length === 1) {
      next[0] = { ...next[0], isDefault: true }
    }
    onAccountsChange(next)
    setShowAddForm(false)
    setEditingIndex(null)
  }

  if (!open) return null

  const himalayaReady = status?.installed ?? false

  return (
    <div
      className="kv-modal-backdrop"
      data-tauri-drag-region="false"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div
        className="kv-modal kv-connector-detail max-w-xl"
        role="dialog"
        aria-modal="true"
        data-tauri-drag-region="false"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="kv-connector-detail-header">
          <div className="flex min-w-0 flex-1 items-center gap-2">
            <EmailBrandIcon size={20} className="shrink-0 opacity-90" />
            <div className="truncate text-sm font-medium">Email</div>
          </div>
          <IconButton
            size="xs"
            className="shrink-0"
            onClick={onClose}
            data-tauri-drag-region="false"
            label={t.connectorsDetailClose}
          >
            <X size={14} />
          </IconButton>
        </div>

        <div className="kv-connector-detail-body custom-scrollbar space-y-4 p-4">
          <section className="kv-panel space-y-2 p-3">
            <div className="text-sm font-medium">{t.connectorsEmailRuntimeTitle}</div>
            {status?.installed ? (
              <div className="kv-row-desc text-[12px] text-emerald-600 dark:text-emerald-400">
                {t.connectorsEmailRuntimeReady}
                {status.version ? ` (${status.version})` : ''}
              </div>
            ) : (
              <div className="kv-row-desc text-[12px] opacity-80">{t.connectorsEmailRuntimeMissing}</div>
            )}
            {!himalayaReady && (
              <Button
                variant="primary"
                size="sm"
                disabled={installing}
                onClick={() => void handleInstall()}
                data-tauri-drag-region="false"
              >
                {installing ? (
                  <>
                    <Loader2 size={10} className="animate-spin" />
                    {t.connectorsEmailInstalling}
                  </>
                ) : (
                  t.connectorsEmailInstall
                )}
              </Button>
            )}
            {installError && (
              <div className="kv-row-desc text-[12px] text-red-500 dark:text-red-400">{installError}</div>
            )}
          </section>

          {accounts.length > 0 && (
            <section className="space-y-2">
              <div className="text-sm font-medium">{t.connectorsEmailAccountsTitle}</div>
              <div className="space-y-2">
                {accounts.map((account, index) => (
                  <div key={`${account.id}-${account.email}`} className="kv-panel flex items-center gap-2 p-2">
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-[13px] font-medium">{account.email}</div>
                      <div className="kv-row-desc truncate text-[11px] opacity-70">
                        {account.imapHost} · {account.smtpHost}
                        {account.isDefault ? ` · ${t.connectorsEmailDefaultBadge}` : ''}
                      </div>
                    </div>
                    <Button
                      size="sm"
                      disabled={!himalayaReady}
                      onClick={() => {
                        setEditingIndex(index)
                        setShowAddForm(true)
                      }}
                      data-tauri-drag-region="false"
                    >
                      {t.connectorsEmailEdit}
                    </Button>
                    <Button
                      variant="danger"
                      size="sm"
                      onClick={() => removeAccount(index)}
                      data-tauri-drag-region="false"
                    >
                      <Trash2 size={10} />
                    </Button>
                  </div>
                ))}
              </div>
            </section>
          )}

          {himalayaReady && !showAddForm && (
            <Button
              size="sm"
              onClick={() => {
                setEditingIndex(null)
                setShowAddForm(true)
              }}
              data-tauri-drag-region="false"
            >
              <Plus size={10} />
              {t.connectorsEmailAddAccount}
            </Button>
          )}

          {himalayaReady && showAddForm && (
            <section className="kv-panel p-3">
              <div className="mb-2 text-sm font-medium">
                {editingIndex !== null ? t.connectorsEmailEditAccount : t.connectorsEmailAddAccount}
              </div>
              <EmailConnectorForm
                lang={lang}
                initial={editingIndex !== null ? accounts[editingIndex] : undefined}
                existingAccounts={accounts.filter((_, index) => index !== editingIndex)}
                onSave={saveAccount}
                onCancel={() => {
                  setShowAddForm(accounts.length === 0)
                  setEditingIndex(null)
                }}
              />
            </section>
          )}
        </div>
      </div>
    </div>
  )
}
