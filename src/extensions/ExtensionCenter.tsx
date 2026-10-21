import { Component, type ReactNode } from 'react'
import { useLang } from '../components/i18n'
import { Button } from '../components/Button'
import { ErrorNotice } from '../components/AppNotice'
import { Toggle } from '../settings/public/controls'
import type { ExtensionPageProps } from '../../extensions/sdk'
import { bindExtensionHost, catalog, clearExtensionError, extensionPage, setExtensionEnabled, useExtensions } from './runtime'

class ExtensionBoundary extends Component<{ children: ReactNode }, { error: string }> {
  state = { error: '' }
  static getDerivedStateFromError(error: unknown) { return { error: String(error) } }
  render() { return this.state.error ? <div className="p-6"><p>扩展运行失败，请返回扩展管理关闭后重试。</p><ErrorNotice message={this.state.error} /></div> : this.props.children }
}
export function ExtensionCenter({ id, host, onOpen, heading }: { heading: ReactNode; id: string | null; host: ExtensionPageProps; onOpen: (id: string) => void }) {
  const zh = useLang() === 'zh'
  const state = useExtensions()
  const Page = id && state.enabled.includes(id) ? extensionPage(id) : undefined
  if (Page) return <ExtensionBoundary key={id}><Page {...host} {...bindExtensionHost(id!, host)} /></ExtensionBoundary>
  return <>{host.headerControls}<div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto px-5 py-6 sm:px-8">
    <div className="mx-auto max-w-[880px]">
      {heading}
      <p className="mt-2 text-sm text-[var(--color-muted-foreground)]">{zh ? '发现并启用 Kivio 扩展，让更多功能按需加入。关闭扩展会保留已有资料。' : 'Discover and enable Kivio extensions. Disabling an extension keeps your data.'}</p>
      <ErrorNotice message={state.error} />
      {!state.ready && <p className="mt-6" role="status">正在读取扩展…</p>}
      <div className="mt-8 divide-y divide-[var(--color-border)] border-t border-[var(--color-border)]">
        {catalog.map(({ manifest }) => {
          const enabled = state.enabled.includes(manifest.id)
          return <div key={manifest.id} className="flex flex-wrap items-center gap-4 py-5">
            <div className="min-w-48 flex-1"><div className="flex flex-wrap items-baseline gap-3"><h2 className="font-medium">{manifest.name}</h2><span className="text-xs text-[var(--color-muted-foreground)]">{manifest.version}</span></div><p className="mt-1 text-sm text-[var(--color-muted-foreground)]">{manifest.description}</p></div>
            <span className="text-xs text-[var(--color-muted-foreground)]">{enabled ? (zh ? '已启用' : 'Enabled') : (zh ? '未启用' : 'Disabled')}</span>
            {enabled && extensionPage(manifest.id) && <Button variant="ghost" disabled={state.busy} onClick={() => { clearExtensionError(); onOpen(manifest.id) }}>{zh ? '打开' : 'Open'}</Button>}
            <Toggle checked={enabled} disabled={!state.ready || state.busy} onChange={() => { void setExtensionEnabled(manifest.id, !enabled).catch(() => {}) }} ariaLabel={`${enabled ? '关闭' : '启用'} ${manifest.name}`} />
          </div>
        })}
      </div>
    </div>
  </div></>
}
