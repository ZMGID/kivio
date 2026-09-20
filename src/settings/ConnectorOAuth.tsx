import { useEffect, useState } from 'react'
import { api, type OAuthDevicePrompt } from '../api/tauri'
import { i18n, type Lang } from '../components/i18n'
import { Button } from '../components/Button'

export function OAuthDeviceDialog({ prompt, onCancel, lang }: { prompt: OAuthDevicePrompt | null; onCancel: () => void; lang: Lang }) {
  const t = i18n[lang]
  const [copied, setCopied] = useState(false)
  const [error, setError] = useState('')
  useEffect(() => { setCopied(false); setError('') }, [prompt?.userCode])
  useEffect(() => {
    if (!prompt) return
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopImmediatePropagation(); onCancel() } }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [prompt, onCancel])
  if (!prompt) return null
  return <div className="kv-modal-backdrop" style={{ zIndex: 10000 }}>
    <section className="kv-modal max-w-md space-y-4 p-6" role="dialog" aria-modal="true" aria-label={t.connectorsDeviceTitle}>
      <h2 className="text-lg font-semibold">{t.connectorsDeviceTitle}</h2>
      <p className="text-sm">{t.connectorsDeviceHint}</p>
      <div className="select-all rounded-lg border p-3 text-center font-mono text-2xl tracking-widest">{prompt.userCode}</div>
      <p className="text-xs opacity-70">{t.connectorsDeviceScope}</p>
      <div className="flex flex-wrap gap-2">
        <Button onClick={() => void api.chatWriteClipboardText(prompt.userCode).then(() => setCopied(true)).catch(() => setError(t.connectorsDeviceCopyFailed))}>{copied ? t.connectorsDeviceCopied : t.connectorsDeviceCopy}</Button>
        <Button onClick={() => void api.openExternal(prompt.verificationUri).catch(() => setError(t.connectorsDeviceOpenFailed))}>{t.connectorsDeviceOpen}</Button>
        <Button variant="ghost" onClick={onCancel}>{t.cancel}</Button>
      </div>
      {error && <p role="alert" className="text-sm text-red-500">{error}</p>}
      <p className="text-xs opacity-70">{t.connectorsDeviceWaiting}</p>
    </section>
  </div>
}
