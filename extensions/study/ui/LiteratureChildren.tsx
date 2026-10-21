import { Fragment, useEffect, useState, type ReactNode, type MouseEvent } from 'react'
import { ChevronDown, ChevronRight, FileText, Highlighter, Languages, NotebookPen } from 'lucide-react'
import { Button, IconButton } from '../../sdk/ui'
import { ErrorNotice } from '../../sdk/ui'
import type { ReadingDocument, ReadingNote, ReadingOpenTarget } from '../api/reading'
import { readingOperationsApi, type ReadingVersion } from '../api/readingOperations'
import { readingLanguageName } from './readingLanguages'

export type LiteratureChild = { id: string; title: string; description: string; target: ReadingOpenTarget }

/** Expanded contents are derived from the paper's existing notes and immutable
 * PDF outputs. No second attachment registry or copied note state. */
export default function LiteratureChildren({ doc, active, selected, onSelect, onOpen, onContextMenu }: {
  doc: ReadingDocument
  active: boolean
  selected?: string
  onSelect: (child: LiteratureChild) => void
  onOpen: (target: ReadingOpenTarget) => void
  onContextMenu: (child: LiteratureChild, event: MouseEvent) => void
}) {
  const [versions, setVersions] = useState<ReadingVersion[]>([])
  const [loading, setLoading] = useState(doc.hasPdf !== false)
  const [error, setError] = useState('')
  const [retry, setRetry] = useState(0)
  const [expanded, setExpanded] = useState<string[]>([])
  useEffect(() => {
    if (!active || doc.hasPdf === false) return
    let live = true
    setLoading(true); setError('')
    void readingOperationsApi.versions(doc.id).then(value => {
      if (live) setVersions(value)
    }).catch(error => { if (live) setError(String(error)) })
      .finally(() => { if (live) setLoading(false) })
    return () => { live = false }
  }, [doc.id, doc.hasPdf, active, retry])
  const toggle = (id: string) => setExpanded(old => old.includes(id) ? old.filter(value => value !== id) : [...old, id])
  const noteChild = (note: ReadingNote): LiteratureChild => ({
    id: `note:${note.id}`,
    title: note.content.replace(/[#*_`>\n]+/g, ' ').trim().slice(0, 90) || '未命名笔记',
    description: note.anchor ? `第 ${note.anchor.page} 页 · ${note.kind === 'highlight' ? '标注' : '笔记'}` : note.kind === 'answer' ? '保存的回答' : '阅读笔记',
    target: { noteId: note.id, page: note.anchor?.page, view: note.anchor ? note.anchor.view ?? { kind: 'source' } : undefined },
  })
  const row = (child: LiteratureChild, icon: ReactNode, depth: number, annotations: ReadingNote[] = []): ReactNode => <Fragment key={child.id}>
    <tr className="literature-child-row" aria-selected={selected === child.id} onContextMenu={event => onContextMenu(child, event)}>
      <td colSpan={2}>
        <div className={`literature-child-line depth-${depth}`}>
          {annotations.length ? <IconButton label={`${expanded.includes(child.id) ? '收起' : '展开'}${child.title}的标注`} size="xs" variant="ghost" aria-expanded={expanded.includes(child.id)} onClick={() => toggle(child.id)}>
            {expanded.includes(child.id) ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
          </IconButton> : <span className="literature-tree-spacer" />}
          <button type="button" className="literature-child-content" onClick={() => onSelect(child)} onDoubleClick={() => onOpen(child.target)}>
            <span className="literature-child-icon">{icon}</span>
            <span className="literature-child-title">{child.title}</span>
            <span className="literature-child-description">{child.description}</span>
            {annotations.length > 0 && <span className="literature-child-count">{annotations.length}</span>}
          </button>
        </div>
      </td>
    </tr>
    {expanded.includes(child.id) && annotations.map(note => row(noteChild(note), <Highlighter size={14} />, depth + 1))}
  </Fragment>
  const annotations = (kind: 'source' | 'mono' | 'dual', runId?: string) => doc.notes.filter(note => note.kind === 'highlight' && note.anchor && (note.anchor.view?.kind ?? 'source') === kind && (kind === 'source' || note.anchor.view?.runId === runId))
  const notes = doc.notes.filter(note => note.kind !== 'highlight' || !note.anchor || doc.hasPdf === false || (!loading && note.anchor.view && !versions.some(version => version.runId === note.anchor?.view?.runId)))
  return <>
    <ErrorNotice message={error} />
    {doc.hasPdf !== false && row({ id: 'source', title: '原文 PDF', description: '原始文件', target: { view: { kind: 'source' } } }, <FileText size={15} />, 1, annotations('source'))}
    {versions.map((version, index) => (['mono', 'dual'] as const).map(kind => {
      const language = readingLanguageName(version.targetLang)
      const date = version.createdAt ? new Date(version.createdAt).toLocaleDateString('zh-CN', { month: 'numeric', day: 'numeric' }) : ''
      return row({ id: `${kind}:${version.runId}`, title: kind === 'mono' ? `${language}译文` : `${language}双语 PDF`, description: [version.page ? `第 ${version.page} 页` : '全文', index === 0 ? '最新' : date || '历史版本'].join(' · '), target: { view: { kind, runId: version.runId } } }, <Languages size={15} />, 1, annotations(kind, version.runId))
    }))}
    {loading && <tr className="literature-child-row"><td colSpan={2}><p className="literature-child-status" role="status">正在读取翻译版本…</p></td></tr>}
    {error && <tr className="literature-child-row"><td colSpan={2}><div className="literature-child-status"><Button size="sm" variant="ghost" onClick={() => setRetry(value => value + 1)}>重新读取翻译版本</Button></div></td></tr>}
    {notes.map(note => row(noteChild(note), <NotebookPen size={15} />, 1))}
  </>
}
