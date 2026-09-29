import { useEffect, useRef, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { FolderOpen, Loader2, RefreshCw, Trash2 } from 'lucide-react'
import { marketplaceApi, type Marketplace } from '../../api/market'
import { Button, IconButton } from '../../components/Button'
import { Input } from '../../settings/public/controls'

export function MarketplaceDialog({ zh, mode, markets, onClose, onChanged }: {
  zh: boolean
  mode: 'add' | 'manage'
  markets: Marketplace[]
  onClose: () => void
  onChanged: (next: Marketplace[], added: boolean) => void
}) {
  const dialog = useRef<HTMLDialogElement>(null)
  const pending = useRef(false)
  const mounted = useRef(true)
  const [source, setSource] = useState('')
  const [busy, setBusy] = useState('')
  const [error, setError] = useState('')
  const [removing, setRemoving] = useState<string | null>(null)
  const text = (cn: string, en: string) => zh ? cn : en
  useEffect(() => {
    mounted.current = true
    dialog.current?.showModal()
    return () => { mounted.current = false }
  }, [])
  const run = async (id: string, action: () => Promise<Marketplace[]>, added = false) => {
    if (pending.current) return
    pending.current = true
    setBusy(id)
    setError('')
    try {
      const next = await action()
      if (mounted.current) { setRemoving(null); onChanged(next, added) }
    } catch (e) { if (mounted.current) setError(String(e)) }
    finally { pending.current = false; if (mounted.current) setBusy('') }
  }
  const chooseFolder = async () => {
    try {
      const selected = await open({ directory: true, multiple: false })
      if (mounted.current && typeof selected === 'string') setSource(selected)
    } catch (e) { if (mounted.current) setError(String(e)) }
  }
  return <dialog ref={dialog} className="kv-modal kv-market-import" aria-labelledby="marketplace-dialog-title"
    onCancel={e => { e.preventDefault(); if (!busy) onClose() }}>
    <form onSubmit={e => { e.preventDefault(); if (source.trim()) void run('add', () => marketplaceApi.add(source.trim()), true) }}>
      <h2 id="marketplace-dialog-title">{mode === 'add' ? text('添加插件市场', 'Add marketplace') : text('管理插件市场', 'Manage marketplaces')}</h2>
      {mode === 'add' ? <>
        <p className="kv-market-muted">{text('添加 Claude Code 格式的市场，浏览并安装其中的插件。', 'Add a Claude Code marketplace to browse and install its plugins.')}</p>
        <label className="kv-market-field"><span>{text('市场来源', 'Marketplace source')}</span>
          <Input autoFocus aria-label={text('市场来源', 'Marketplace source')} value={source} onChange={setSource} disabled={!!busy}
            placeholder={text('owner/repo 或 https://…', 'owner/repo or https://…')} />
        </label>
        <div className="kv-market-source-shortcuts">
          <Button size="sm" disabled={!!busy} onClick={() => setSource('anthropics/claude-plugins-official')}>{text('Claude 官方市场', 'Claude official marketplace')}</Button>
          <Button size="sm" disabled={!!busy} onClick={() => void chooseFolder()}><FolderOpen size={14} />{text('本地目录', 'Local folder')}</Button>
        </div>
        <p className="kv-market-muted">{text('支持 GitHub 简写、HTTPS Git 仓库、JSON 地址或本地市场。Git 来源可用 #分支 指定版本。', 'Use GitHub owner/repo, an HTTPS Git repository, a JSON URL or a local market. Append #ref to pin a Git revision.')}</p>
      </> : <div className="kv-market-sources custom-scrollbar">
        {!markets.length && <p className="kv-market-muted">{text('还没有添加市场。通过“添加”菜单添加第一个市场。', 'No marketplaces yet. Use the Add menu to add one.')}</p>}
        {markets.map(market => <div className="kv-market-source" key={market.id}>
          <div className="kv-market-source-heading"><strong>{market.name}</strong><span className="kv-market-muted">{market.plugins.length} {text('个插件', 'plugins')}</span></div>
          <p className="kv-market-muted kv-market-source-url">{market.source}</p>
          {removing === market.id ? <>
            <p className="kv-market-muted">{text('移除此市场来源？已安装的插件会保留。', 'Remove this source? Installed plugins will be kept.')}</p>
            <div className="kv-market-dialog-actions"><Button size="sm" disabled={!!busy} onClick={() => setRemoving(null)}>{text('取消', 'Cancel')}</Button>
              <Button size="sm" disabled={!!busy} onClick={() => void run(market.id, () => marketplaceApi.remove(market.id))}>{text('确认移除', 'Confirm removal')}</Button></div>
          </> : <div className="kv-market-dialog-actions">
            <IconButton label={text(`刷新 ${market.name}`, `Refresh ${market.name}`)} disabled={!!busy} onClick={() => void run(market.id, () => marketplaceApi.refresh(market.id))}><RefreshCw size={14} className={busy === market.id ? 'animate-spin' : undefined} /></IconButton>
            <IconButton label={text(`移除 ${market.name}`, `Remove ${market.name}`)} disabled={!!busy} onClick={() => setRemoving(market.id)}><Trash2 size={14} /></IconButton>
          </div>}
        </div>)}
      </div>}
      {busy && <p role="status" className="kv-market-muted">{text('正在读取市场，请稍候…', 'Reading marketplace, please wait…')}</p>}
      {error && <p className="kv-market-error" role="alert">{error}</p>}
      <footer className="kv-market-dialog-actions">
        <Button disabled={!!busy} variant="ghost" onClick={onClose}>{mode === 'add' ? text('取消', 'Cancel') : text('完成', 'Done')}</Button>
        {mode === 'add' && <Button type="submit" variant="primary" disabled={!!busy || !source.trim()}>{busy && <Loader2 size={14} className="animate-spin" />}{text('添加市场', 'Add marketplace')}</Button>}
      </footer>
    </form>
  </dialog>
}
