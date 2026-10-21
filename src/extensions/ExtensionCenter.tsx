import { Component, type ReactNode } from 'react'
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
export function ExtensionCenter({ id, host, onOpen }: { id: string | null; host: ExtensionPageProps; onOpen: (id: string) => void }) {
  const state = useExtensions()
  const Page = id && state.enabled.includes(id) ? extensionPage(id) : undefined
  if (Page) return <ExtensionBoundary key={id}><Page {...host} {...bindExtensionHost(id!, host)} /></ExtensionBoundary>
  return <>{host.headerControls}<div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto px-8 py-8">
    <div className="mx-auto max-w-3xl">
      <h1 className="text-xl font-semibold text-neutral-900 dark:text-neutral-100">扩展管理</h1>
      <p className="mt-2 text-sm text-neutral-500">按需启用功能。关闭扩展会保留已有资料。</p>
      <ErrorNotice message={state.error} />
      {!state.ready && <p className="mt-6" role="status">正在读取扩展…</p>}
      <div className="mt-8 divide-y divide-neutral-200 dark:divide-neutral-800">
        {catalog.map(({ manifest }) => {
          const enabled = state.enabled.includes(manifest.id)
          return <div key={manifest.id} className="flex flex-wrap items-center gap-4 py-5">
            <div className="min-w-48 flex-1"><h2 className="font-medium">{manifest.name}</h2><p className="mt-1 text-sm text-neutral-500">{manifest.description}</p></div>
            {enabled && extensionPage(manifest.id) && <Button variant="ghost" disabled={state.busy} onClick={() => { clearExtensionError(); onOpen(manifest.id) }}>打开</Button>}
            <Toggle checked={enabled} disabled={!state.ready || state.busy} onChange={() => { void setExtensionEnabled(manifest.id, !enabled).catch(() => {}) }} ariaLabel={`${enabled ? '关闭' : '启用'} ${manifest.name}`} />
          </div>
        })}
      </div>
    </div>
  </div></>
}
