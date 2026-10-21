import { useEffect, useState } from 'react'
import { save } from '@tauri-apps/plugin-dialog'
import { Button, IconButton } from '../../sdk/ui'
import { X } from 'lucide-react'
import { ErrorNotice, OperationNotice } from '../../sdk/ui'
import { readingWritingApi, type WordConnection } from '../api/readingWriting'
import type { ReadingDocument } from '../api/reading'
import { flushReadingItems } from './readingLibrary'

export function StudyWriting({ documents, onClose }: { documents: ReadingDocument[]; onClose: () => void }) {
  const [connection, setConnection] = useState<WordConnection | null>(null)
  const [checking, setChecking] = useState(true)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  useEffect(() => { let alive = true; readingWritingApi.status().then(s => { if (alive) setConnection(s) }, e => { if (alive) setError(String(e)) }).finally(() => { if (alive) setChecking(false) }); return () => { alive = false } }, [])
  async function run(action: () => Promise<void>) { setBusy(true); setError(''); setNotice(''); try { await action() } catch (e) { setError(String(e)) } finally { setBusy(false) } }
  const active = documents.filter(d => !d.trashedAt)
  return <section className="study-writing custom-scrollbar" aria-label="写作引用">
    <div className="literature-detail-heading"><h2>写作引用</h2><IconButton label="关闭写作引用" variant="ghost" onClick={onClose} disabled={busy}><X size={16} /></IconButton></div>
    <ErrorNotice message={error} /><OperationNotice message={notice} />
    <p className="study-writing-intro literature-muted">连接 Word，或将当前列表中的文献带入 LaTeX。</p>
    <section className="study-writing-section" aria-label="Word 引用"><h3>在 Word 中引用</h3>
    <p className="literature-muted">Word 直接读取本机 Study 文献库。插入多篇引用、切换格式，并刷新编号与参考文献。</p>
    <p className="literature-muted">首次安装会向当前用户安装本机 HTTPS 证书和 Word 加载项，系统可能要求确认。安装后重启 Word，在「开始 → 加载项」打开 Kivio Study。需要桌面 Word 支持 WordApi 1.3。</p>
    {!connection && !checking && <Button disabled={busy} onClick={() => void run(async () => {
      setChecking(true)
      try { setConnection(await readingWritingApi.status()) } finally { setChecking(false) }
    })}>重试检查</Button>}
    <div className="study-writing-actions">
      <Button disabled={busy || !connection} onClick={() => void run(async () => { setConnection(await readingWritingApi.install()); setNotice('加载项已注册，请重启 Word 并打开 Kivio Study。首次连接需粘贴连接码。') })}>{connection?.installed ? '重新安装 Word 加载项' : '安装 Word 加载项'}</Button>
      <Button disabled={busy || !connection} onClick={() => void run(async () => setConnection(await (connection?.running ? readingWritingApi.stop() : readingWritingApi.start())))}>{connection?.running ? '关闭连接并撤销连接码' : '开启本机连接'}</Button>
      <Button disabled={busy || !connection?.running} onClick={() => void run(async () => { await navigator.clipboard.writeText(connection!.pairingCode); setNotice('连接码已复制，请粘贴到 Word 的 Kivio Study 面板。') })}>复制连接码</Button>
    </div>
    <p className="literature-muted">{connection ? connection.running ? '本机连接已开启。写作时请保持 Kivio 运行；资料修改保存后，在 Word 点击刷新。' : '本机连接已关闭。文档中已有的引用仍保留。' : checking ? '正在检查本机连接…' : '连接状态检查失败，请重试。'}</p>
    </section>
    <section className="study-writing-section" aria-label="LaTeX 引用"><h3>在 LaTeX 中引用</h3>
    <p className="literature-muted">{active.length ? `导出 ${active.length} 篇文献为 BibTeX。` : '当前列表没有可导出的文献。返回文献库后选择材料。'}引用键由文献 ID 固定生成，修改标题或作者后保持不变。中文文献建议使用 XeLaTeX / LuaLaTeX 与 Biber。</p>
    <div className="study-writing-actions">
      <Button disabled={busy || !active.length} onClick={() => void run(async () => {
        const path = await save({ defaultPath: 'kivio-references.bib', filters: [{ name: 'BibTeX', extensions: ['bib'] }] })
        if (!path) return
        const ids = active.map(d => d.id)
        await flushReadingItems(ids)
        await readingWritingApi.exportBib(ids, path)
        setNotice(`已导出 ${ids.length} 篇文献。`)
      })}>导出 .bib</Button>
      <Button disabled={busy || !active.length} onClick={() => void run(async () => { await navigator.clipboard.writeText(`\\cite{${active.map(d => `kivio${d.id.replaceAll('-', '')}`).join(',')}}`); setNotice('LaTeX 引用命令已复制。') })}>复制 LaTeX 引用</Button>
    </div>
    <p className="literature-muted">请核对作者、年份、卷期和页码。英文作者可按「姓, 名」录入，以便引用引擎正确缩写姓名。</p>
    </section>
  </section>
}
