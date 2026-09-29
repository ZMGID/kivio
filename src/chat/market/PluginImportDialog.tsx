import { useEffect, useRef, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { FolderOpen } from 'lucide-react'
import { packageApi, type PluginPackage } from '../../api/pluginPackages'
import { Button } from '../../components/Button'
import { Input } from '../../settings/public/controls'

export function PluginImportDialog({ zh, kind, onClose, onImported }: {
  zh: boolean
  kind: 'local' | 'git'
  onClose: () => void
  onImported: (plugin: PluginPackage) => void
}) {
  const dialog = useRef<HTMLDialogElement>(null)
  const pending = useRef(false)
  const mounted = useRef(true)
  const [source, setSource] = useState('')
  const [subdirectory, setSubdirectory] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const text = (cn: string, en: string) => zh ? cn : en
  useEffect(() => {
    mounted.current = true
    dialog.current?.showModal()
    return () => { mounted.current = false }
  }, [])
  const submit = async () => {
    if (pending.current || !source.trim()) return
    pending.current = true
    setBusy(true)
    setError('')
    try {
      const plugin = await packageApi.import(source.trim(), subdirectory.trim() || undefined)
      if (mounted.current) onImported(plugin)
    } catch (e) { if (mounted.current) setError(String(e)) }
    finally { pending.current = false; if (mounted.current) setBusy(false) }
  }
  const chooseFolder = async () => {
    try {
      const path = await open({ directory: true, multiple: false })
      if (mounted.current && typeof path === 'string') setSource(path)
    } catch (e) { if (mounted.current) setError(String(e)) }
  }
  return <dialog ref={dialog} className="kv-modal kv-market-import" aria-labelledby="plugin-import-title"
    onCancel={e => { e.preventDefault(); if (!busy) onClose() }}>
    <form onSubmit={e => { e.preventDefault(); void submit() }}>
      <h2 id="plugin-import-title">{kind === 'local' ? text('从本地目录导入', 'Import from a folder') : text('从 Git 仓库导入', 'Import from Git')}</h2>
      <p className="kv-market-muted">{text('支持 Kivio、Claude Code 和 Codex 格式的插件。', 'Supports Kivio, Claude Code and Codex plugins.')}</p>
      <label className="kv-market-field">
        <span>{kind === 'local' ? text('插件目录', 'Plugin folder') : text('仓库地址', 'Repository URL')}</span>
        <Input autoFocus aria-label={text('插件来源', 'Plugin source')} value={source} onChange={setSource} disabled={busy}
          placeholder={kind === 'local' ? text('选择包含插件的文件夹', 'Choose a plugin folder') : 'https://github.com/owner/repository'} />
      </label>
      {kind === 'local' && <Button disabled={busy} onClick={() => void chooseFolder()}><FolderOpen size={14} />{text('选择目录', 'Choose folder')}</Button>}
      <label className="kv-market-field">
        <span>{text('子目录（可选）', 'Subdirectory (optional)')}</span>
        <Input aria-label={text('插件子目录', 'Plugin subdirectory')} value={subdirectory} onChange={setSubdirectory} disabled={busy} placeholder="plugins/my-plugin" />
      </label>
      <p className="kv-market-muted">{text('导入后默认停用。启用会加载插件能力并允许执行 Hook 脚本；依赖需自行安装。仅供内置 Kivio Agent 使用。', 'Imports start disabled. Enabling loads capabilities and permits hook scripts; install dependencies separately. For the built-in Kivio Agent.')}</p>
      {error && <p className="kv-market-error" role="alert">{error}</p>}
      <footer className="kv-market-dialog-actions">
        <Button disabled={busy} variant="ghost" onClick={onClose}>{text('取消', 'Cancel')}</Button>
        <Button type="submit" variant="primary" disabled={busy || !source.trim()}>{busy ? text('正在导入…', 'Importing…') : text('导入', 'Import')}</Button>
      </footer>
    </form>
  </dialog>
}
