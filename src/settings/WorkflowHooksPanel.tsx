import { ErrorNotice } from '../components/AppNotice'
import { useEffect, useState } from 'react'
import { packageApi } from '../api/pluginPackages'
import { Button } from '../components/Button'
import type { Lang } from '../components/i18n'

export function WorkflowHooksPanel({ lang }: { lang: Lang }) {
  const zh = lang === 'zh'
  const [text, setText] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [saved, setSaved] = useState(false)
  useEffect(() => { void packageApi.getHooks().then(config => setText(JSON.stringify(config, null, 2))).catch(e => setError(String(e))) }, [])
  return <section className="mb-5 space-y-3 rounded-xl border border-neutral-200 p-4">
    <h3 className="text-sm font-semibold">{zh ? '工作流 Hooks' : 'Workflow hooks'}</h3>
    <p className="text-xs text-neutral-500">{zh
      ? '这些钩子在执行边界等待结果，可注入上下文、拒绝工具或改写参数。下方原有生命周期钩子继续作为通知使用。配置单独保存，新一轮对话生效，仅用于内置 Agent。'
      : 'These hooks await results to inject context, deny tools or update arguments. Existing lifecycle hooks below remain notifications. Saved separately; applies on the next built-in Agent run.'}</p>
    <textarea className="kv-input w-full font-mono text-xs" rows={10} aria-label={zh ? '工作流 Hook JSON' : 'Workflow hook JSON'} value={text} onChange={e => { setText(e.target.value); setSaved(false) }} disabled={busy} spellCheck={false} />
    {error && <ErrorNotice message={error} />}
    {saved && <p role="status" className="text-xs text-green-600">{zh ? '已保存' : 'Saved'}</p>}
    <Button disabled={busy || !text.trim()} onClick={() => { setBusy(true); setError(''); setSaved(false); void (async () => {
      try { await packageApi.saveHooks(JSON.parse(text)); setSaved(true) } catch (e) { setError(String(e)) } finally { setBusy(false) }
    })() }}>{zh ? '保存工作流 Hooks' : 'Save workflow hooks'}</Button>
  </section>
}
