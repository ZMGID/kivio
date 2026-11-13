import { useMemo, useState } from 'react'
import { useT, type I18n } from '../components/i18n'
import { Check, Copy, Search, Terminal } from 'lucide-react'
import { copyToClipboard } from '../utils/clipboard'

import { parseQuotas, type CliReport } from './cliCommandReportData'

const titleKeys: Record<string, keyof I18n> = {
  usage: 'chatCliModelQuota', quota: 'chatCliModelQuota', help: 'chatCliCommandGuide', skills: 'chatCliSkills',
  agents: 'chatCliAgents', model: 'chatCliCurrentModel', effort: 'chatCliEffort', config: 'chatCliConfig',
  settings: 'chatCliConfig', permissions: 'chatCliPermissions', hooks: 'chatCliHooks', changelog: 'chatCliChangelog',
}

function reportTitle(command: string, t: I18n): string | undefined {
  if (command === 'credits') return 'AI Credits'
  const key = titleKeys[command]
  return key ? t[key] : undefined
}

function CopyButton({ value, label: labelProp }: { value: string; label?: string }) {
  const t = useT()
  const label = labelProp ?? t.chatCliCopyRaw
  const [copied, setCopied] = useState(false)
  return <button type="button" aria-label={copied ? t.chatMessageCopied : label} title={label}
    className="shrink-0 rounded-md p-1.5 text-neutral-400 hover:bg-neutral-900/5 hover:text-neutral-700 focus-visible:outline focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
    onClick={async () => { if (await copyToClipboard(value)) setCopied(true) }}>
    {copied ? <Check size={14} /> : <Copy size={14} />}
  </button>
}

function resetLabel(reset: string): string {
  return new Intl.DateTimeFormat('zh-CN', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false }).format(new Date(reset))
}

function reportRows(output: string): { name: string; value: string }[] {
  try {
    const value: unknown = JSON.parse(output)
    if (value && typeof value === 'object' && !Array.isArray(value)) {
      return Object.entries(value).map(([name, v]) => ({ name, value: typeof v === 'string' ? v : JSON.stringify(v, null, 2) }))
    }
  } catch { /* native text format */ }
  return output.split(/\r?\n/).filter(line => line.trim()).map(line => {
    const match = line.match(/^(.+?)(?:\t+| — |:\s+)(.*)$/)
    return match ? { name: match[1], value: match[2].replace(/\t/g, ' · ') } : { name: line, value: '' }
  })
}

export function CliCommandReport({ report }: { report: CliReport }) {
  const t = useT()
  const [query, setQuery] = useState('')
  const quota = useMemo(() => ['usage', 'quota'].includes(report.command) ? parseQuotas(report.output) : [], [report])
  const rows = useMemo(() => reportRows(report.output), [report.output])
  const searchable = ['help', 'skills', 'agents'].includes(report.command)
  const visible = rows.filter(row => `${row.name} ${row.value}`.toLowerCase().includes(query.toLowerCase()))
  const groups = [...new Set(quota.map(row => row.group))]
  return <section aria-label={reportTitle(report.command, t) ?? `/${report.command}`} className="not-prose my-3 w-full max-w-[600px] overflow-hidden rounded-xl border border-neutral-900/[0.07] bg-[var(--bg-primary)] text-sm text-neutral-800">
    <header className="flex items-center gap-2 border-b border-neutral-900/5 px-4 py-3">
      <Terminal size={16} className="text-neutral-400" />
      <h3 className="m-0 flex-1 text-sm font-semibold">{reportTitle(report.command, t) ?? t.chatCliResult}</h3>
      <span className="font-mono text-[11px] text-neutral-400">/{report.command}</span>
      <CopyButton value={report.output} />
    </header>
    {quota.length > 0 ? <div className="space-y-5 px-4 py-4">
      {groups.map(group => <div key={group}>
        <h4 className="mb-3 text-xs font-semibold text-neutral-500">{group}</h4>
        <div className="grid grid-cols-1 gap-4 min-[460px]:grid-cols-2">
          {quota.filter(row => row.group === group).map(row => <div key={row.window}>
            <div className="mb-2 flex items-baseline justify-between gap-2">
              <span className="text-xs">{row.window}</span>
              <span className="text-xs text-neutral-400">{t.chatCliRemaining} <strong className="ml-1 text-lg font-semibold tabular-nums text-neutral-800">{row.remaining}<span className="text-xs">%</span></strong></span>
            </div>
            <div role="progressbar" aria-label={t.chatCliRemainingOf.replace('{name}', `${group} ${row.window}`)} aria-valuenow={row.remaining} aria-valuemin={0} aria-valuemax={100}
              className="h-1.5 overflow-hidden rounded-full bg-neutral-100">
              <div className={`h-full rounded-full ${row.remaining <= 10 ? 'bg-amber-500' : 'bg-emerald-500 dark:bg-emerald-400'}`} style={{ width: `${row.remaining}%` }} />
            </div>
            <p className="mb-0 mt-2 text-[11px] text-neutral-400">{t.chatCliResetsAt} <time dateTime={row.reset} title={row.reset}>{resetLabel(row.reset)}</time></p>
          </div>)}
        </div>
      </div>)}
      <p className="m-0 text-[10px] text-neutral-400">{t.chatCliQuotaNote}</p>
    </div> : <>
      {searchable && <div className="flex items-center gap-2 border-b border-neutral-900/5 px-4 py-2">
        <Search size={14} className="text-neutral-400" />
        <input aria-label={t.chatCliFilterLabel} placeholder={t.chatCliFilterPlaceholder} value={query} onChange={event => setQuery(event.target.value)} className="min-w-0 flex-1 bg-transparent py-1 text-xs outline-none placeholder:text-neutral-400" />
        <span className="text-[11px] tabular-nums text-neutral-400">{t.chatCliItemsCount.replace('{n}', String(visible.length))}</span>
      </div>}
      <div className="custom-scrollbar max-h-96 overflow-y-auto px-4">
        {visible.map((row, index) => <div key={`${row.name}:${index}`} className="flex gap-2 border-b border-neutral-900/5 py-3 last:border-0">
          <div className="min-w-0 flex-1">
            <div className={`break-words ${report.command === 'model' || report.command === 'effort' ? 'text-base font-semibold' : 'text-xs font-medium'}`}>{row.name}</div>
            {row.value && <div className="mt-1 whitespace-pre-wrap break-words text-xs leading-relaxed text-neutral-500">{row.value}</div>}
          </div>
          {searchable && (report.command !== 'help' || row.name.startsWith('/')) && <CopyButton value={report.command === 'skills' && !row.name.startsWith('/') ? `/${row.name}` : row.name} label={t.chatCliCopyName.replace('{name}', row.name)} />}
        </div>)}
        {!visible.length && <p className="py-4 text-center text-xs text-neutral-400">{t.chatCliNoMatches}</p>}
      </div>
    </>}
    <details className="border-t border-neutral-900/5 bg-neutral-900/[0.015]">
      <summary className="cursor-pointer px-4 py-2 text-[11px] text-neutral-400">{t.chatCliRawOutput}</summary>
      <pre className="custom-scrollbar m-0 max-h-56 overflow-auto whitespace-pre-wrap break-words px-4 pb-3 text-[11px] text-neutral-500">{report.output}</pre>
    </details>
  </section>
}
