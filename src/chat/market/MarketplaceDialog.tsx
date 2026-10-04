import { useEffect, useRef } from 'react'
import { FolderOpen, Loader2, RefreshCw, Trash2 } from 'lucide-react'
import { Button, IconButton } from '../../components/Button'
import { Input } from '../../settings/public/controls'
import { useWindowStore } from '../../utils/windowStore'
import { bumpMarketplacePicker, marketWindow, runAddMarketplace, runPickMarketplaceFolder, runRefreshMarketplace, runRemoveMarketplace, setMarketRemoving, setMarketSource } from './marketOperations'

export function MarketplaceDialog({ zh, mode, onClose }: {
  zh: boolean
  mode: 'add' | 'manage'
  onClose: () => void
}) {
  const [{ markets, market }] = useWindowStore(marketWindow)
  const dialog = useRef<HTMLDialogElement>(null)
  const text = (cn: string, en: string) => zh ? cn : en
  const busy = market.busy
  const error = mode === 'add' ? market.error : market.manageError
  useEffect(() => {
    dialog.current?.showModal()
    return () => { bumpMarketplacePicker() }
  }, [])
  return <dialog ref={dialog} className="kv-modal kv-market-import" aria-labelledby="marketplace-dialog-title"
    onCancel={e => { e.preventDefault(); onClose() }}>
    <form onSubmit={e => { e.preventDefault(); if (mode === 'add' && market.source.trim()) void runAddMarketplace() }}>
      <h2 id="marketplace-dialog-title">{mode === 'add' ? text('添加插件市场', 'Add marketplace') : text('管理插件市场', 'Manage marketplaces')}</h2>
      {mode === 'add' ? <>
        <p className="kv-market-muted">{text('添加 Claude Code 格式的市场，浏览并安装其中的插件。', 'Add a Claude Code marketplace to browse and install its plugins.')}</p>
        <label className="kv-market-field"><span>{text('市场来源', 'Marketplace source')}</span>
          <Input autoFocus aria-label={text('市场来源', 'Marketplace source')} value={market.source} onChange={setMarketSource} disabled={!!busy}
            placeholder={text('owner/repo 或 https://…', 'owner/repo or https://…')} />
        </label>
        <div className="kv-market-source-shortcuts">
          <Button size="sm" disabled={!!busy} onClick={() => setMarketSource('anthropics/claude-plugins-official')}>{text('Claude 官方市场', 'Claude official marketplace')}</Button>
          <Button size="sm" disabled={!!busy} onClick={() => void runPickMarketplaceFolder()}><FolderOpen size={14} />{text('本地目录', 'Local folder')}</Button>
        </div>
        <p className="kv-market-muted">{text('支持 GitHub 简写、HTTPS Git 仓库、JSON 地址或本地市场。Git 来源可用 #分支 指定版本。', 'Use GitHub owner/repo, an HTTPS Git repository, a JSON URL or a local market. Append #ref to pin a Git revision.')}</p>
      </> : <div className="kv-market-sources custom-scrollbar">
        {!markets.length && <p className="kv-market-muted">{text('还没有添加市场。通过“添加”菜单添加第一个市场。', 'No marketplaces yet. Use the Add menu to add one.')}</p>}
        {markets.map(item => <div className="kv-market-source" key={item.id}>
          <div className="kv-market-source-heading"><strong>{item.name}</strong><span className="kv-market-muted">{item.plugins.length} {text('个插件', 'plugins')}</span></div>
          <p className="kv-market-muted kv-market-source-url">{item.source}</p>
          {market.removing === item.id ? <>
            <p className="kv-market-muted">{text('移除此市场来源？已安装的插件会保留。', 'Remove this source? Installed plugins will be kept.')}</p>
            <div className="kv-market-dialog-actions"><Button size="sm" disabled={!!busy} onClick={() => setMarketRemoving(null)}>{text('取消', 'Cancel')}</Button>
              <Button size="sm" disabled={!!busy} onClick={() => void runRemoveMarketplace(item.id)}>{text('确认移除', 'Confirm removal')}</Button></div>
          </> : <div className="kv-market-dialog-actions">
            <IconButton label={text(`刷新 ${item.name}`, `Refresh ${item.name}`)} disabled={!!busy} onClick={() => void runRefreshMarketplace(item.id)}><RefreshCw size={14} className={busy === item.id ? 'animate-spin' : undefined} /></IconButton>
            <IconButton label={text(`移除 ${item.name}`, `Remove ${item.name}`)} disabled={!!busy} onClick={() => setMarketRemoving(item.id)}><Trash2 size={14} /></IconButton>
          </div>}
        </div>)}
      </div>}
      {busy && <p role="status" className="kv-market-muted">{text('正在读取市场，请稍候…', 'Reading marketplace, please wait…')}</p>}
      {error && <p className="kv-market-error" role="alert">{error}</p>}
      <footer className="kv-market-dialog-actions">
        <Button variant="ghost" onClick={onClose}>{mode === 'add' ? text('取消', 'Cancel') : text('完成', 'Done')}</Button>
        {mode === 'add' && <Button type="submit" variant="primary" disabled={!!busy || !market.source.trim()}>{busy && <Loader2 size={14} className="animate-spin" />}{text('添加市场', 'Add marketplace')}</Button>}
      </footer>
    </form>
  </dialog>
}
