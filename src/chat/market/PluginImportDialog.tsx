import { useEffect, useRef } from 'react'
import { FolderOpen } from 'lucide-react'
import { Button } from '../../components/Button'
import { Input } from '../../settings/public/controls'
import { useWindowStore } from '../../utils/windowStore'
import { bumpImportPicker, marketWindow, runImportPlugin, runPickImportFolder, setImportSource, setImportSubdirectory, type ImportKind } from './marketOperations'

export function PluginImportDialog({ zh, kind, onClose }: {
  zh: boolean
  kind: ImportKind
  onClose: () => void
}) {
  const [{ imports }] = useWindowStore(marketWindow)
  const draft = imports[kind]
  const dialog = useRef<HTMLDialogElement>(null)
  const text = (cn: string, en: string) => zh ? cn : en
  useEffect(() => {
    dialog.current?.showModal()
  }, [])
  useEffect(() => () => { bumpImportPicker(kind) }, [kind])
  return <dialog ref={dialog} className="kv-modal kv-market-import" aria-labelledby="plugin-import-title"
    onCancel={e => { e.preventDefault(); onClose() }}>
    <form onSubmit={e => { e.preventDefault(); void runImportPlugin(kind) }}>
      <h2 id="plugin-import-title">{kind === 'local' ? text('从本地目录导入', 'Import from a folder') : text('从 Git 仓库导入', 'Import from Git')}</h2>
      <p className="kv-market-muted">{text('支持 Kivio、Claude Code 和 Codex 格式的插件。', 'Supports Kivio, Claude Code and Codex plugins.')}</p>
      <label className="kv-market-field">
        <span>{kind === 'local' ? text('插件目录', 'Plugin folder') : text('仓库地址', 'Repository URL')}</span>
        <Input autoFocus aria-label={text('插件来源', 'Plugin source')} value={draft.source} onChange={value => setImportSource(kind, value)} disabled={draft.busy}
          placeholder={kind === 'local' ? text('选择包含插件的文件夹', 'Choose a plugin folder') : 'https://github.com/owner/repository'} />
      </label>
      {kind === 'local' && <Button disabled={draft.busy} onClick={() => void runPickImportFolder(kind)}><FolderOpen size={14} />{text('选择目录', 'Choose folder')}</Button>}
      <label className="kv-market-field">
        <span>{text('子目录（可选）', 'Subdirectory (optional)')}</span>
        <Input aria-label={text('插件子目录', 'Plugin subdirectory')} value={draft.subdirectory} onChange={value => setImportSubdirectory(kind, value)} disabled={draft.busy} placeholder="plugins/my-plugin" />
      </label>
      <p className="kv-market-muted">{text('导入后默认停用。启用会加载插件能力并允许执行 Hook 脚本；依赖需自行安装。仅供内置 Kivio Agent 使用。', 'Imports start disabled. Enabling loads capabilities and permits hook scripts; install dependencies separately. For the built-in Kivio Agent.')}</p>
      {draft.error && <p className="kv-market-error" role="alert">{draft.error}</p>}
      <footer className="kv-market-dialog-actions">
        <Button variant="ghost" onClick={onClose}>{text('取消', 'Cancel')}</Button>
        <Button type="submit" variant="primary" disabled={draft.busy || !draft.source.trim()}>{draft.busy ? text('正在导入…', 'Importing…') : text('导入', 'Import')}</Button>
      </footer>
    </form>
  </dialog>
}
