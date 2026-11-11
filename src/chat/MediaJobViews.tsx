// 单个作品的详情、动作与灯箱。最近一次生成与创作记录的灯箱共用这些部件，行为只有一处实现。
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { Check, ChevronLeft, ChevronRight, Copy, Download, ImagePlus, Play, RefreshCw, Trash2, X } from 'lucide-react'
import type { MediaJob } from '../api/mediaStation'
import { Button, IconButton } from '../components/Button'
import { copyToClipboard } from '../utils/clipboard'
import { MediaCanvas, RatioGlyph, jobCanvas, type Text } from './MediaCanvas'
import { dayLabel, timeLabel } from './mediaFormat'

export type ResultActions = {
  text: Text
  onResume: () => void
  onExport: (index: number) => void
  onFirstFrame: (index: number) => void
  onReuse: () => void
  onEdit: (index: number) => void
  onDelete: () => void
  deleting: boolean
  resuming: boolean
  exporting: (index: number) => boolean
}

/** Status, waiting state, error, prompt and specs of one creation. */
export function JobDetails({ job, text, zh, statusLabel, onCancel, cancelling, headerEnd }: {
  job: MediaJob; text: Text; zh: boolean; statusLabel: string; onCancel: () => void; cancelling: boolean; headerEnd?: ReactNode
}) {
  const [copied, setCopied] = useState(false)
  useEffect(() => {
    if (!copied) return
    const timer = window.setTimeout(() => setCopied(false), 1400)
    return () => window.clearTimeout(timer)
  }, [copied])
  const { request } = job
  return <div className="kv-media-details">
    <div className="kv-media-details-head">
      <span className={`kv-media-status is-${job.status}`}><i aria-hidden="true" />{statusLabel}</span>
      {headerEnd}
    </div>
    {job.status === 'running' && <div className="kv-media-waiting" aria-live="polite">
      <p>{text('正在生成，稍后回来也可以。', 'Generating. You can come back later.')}</p>
      <small>{text('供应商可能继续生成并计费；停止只是不再等待结果。', 'The provider may continue and charge; stopping only ends the wait.')}</small>
      <Button size="sm" onClick={onCancel} disabled={cancelling}>{text('停止等待', 'Stop waiting')}</Button>
    </div>}
    {job.error && <p role="status" className="kv-panel warn kv-media-job-error">{job.error}</p>}
    <div className="kv-media-prompt-block">
      <p className="kv-media-prompt">{request.prompt}</p>
      <IconButton size="xs" variant="ghost" className="kv-media-copy" label={copied ? text('已复制', 'Copied') : text('复制描述', 'Copy prompt')}
        onClick={() => void copyToClipboard(request.prompt).then(setCopied)}>{copied ? <Check size={12} /> : <Copy size={12} />}</IconButton>
    </div>
    <dl className="kv-media-specs">
      <div><dt>{text('模型', 'Model')}</dt><dd title={request.model}>{request.model}</dd></div>
      <div><dt>{text('比例', 'Ratio')}</dt><dd><RatioGlyph ratio={request.aspectRatio} size={12} />{request.aspectRatio}</dd></div>
      {request.kind === 'video' && <div><dt>{text('长度', 'Length')}</dt><dd>{text(`${request.duration} 秒`, `${request.duration} sec`)}</dd></div>}
      <div><dt>{text('创建', 'Created')}</dt><dd>{dayLabel(job.createdAt, zh)} {timeLabel(job.createdAt, zh)}</dd></div>
    </dl>
  </div>
}

/** What can be done with a creation: keep it (save / edit / first frame / fetch again) above, revisit or remove below. */
export function MediaResultActions({ job, text, onResume, onExport, onFirstFrame, onReuse, onEdit, onDelete, deleting, resuming, exporting }: { job: MediaJob } & ResultActions) {
  const running = job.status === 'running'
  const canResume = Boolean(job.providerTaskId) && !running && job.status !== 'completed'
  const numbered = (label: string, index: number) => job.outputs.length > 1 ? `${label} ${index + 1}` : label
  return <div className="kv-media-actions">
    {(job.outputs.length > 0 || canResume) && <div className="kv-media-actions-main">
      {job.outputs.map((output, index) => <Button key={`save-${output.name}`} size="sm" variant="primary" disabled={exporting(index)} onClick={() => onExport(index)}><Download size={14} />{text('另存为', 'Save as')}</Button>)}
      {job.request.kind === 'image' && job.outputs.map((output, index) => <Button key={`edit-${output.name}`} size="sm" onClick={() => onEdit(index)}><ImagePlus size={14} />{numbered(text('继续编辑', 'Continue editing'), index)}</Button>)}
      {job.request.kind === 'image' && job.outputs.map((output, index) => <Button key={`frame-${output.name}`} size="sm" onClick={() => onFirstFrame(index)}><Play size={14} />{text('用作视频首帧', 'Use as first frame')}</Button>)}
      {canResume && <Button size="sm" onClick={onResume} disabled={resuming}><RefreshCw size={14} />{text('继续获取结果', 'Fetch result again')}</Button>}
    </div>}
    <div className="kv-media-actions-more">
      <Button size="sm" variant="ghost" onClick={onReuse}><RefreshCw size={14} />{job.request.kind === 'video' ? text('修改参数 / 重新生成', 'Change settings / regenerate') : text('复用参数', 'Reuse settings')}</Button>
      <Button size="sm" variant="danger" onClick={onDelete} disabled={deleting || running} title={running ? text('请先停止等待，再删除记录和源文件。', 'Stop waiting before deleting the record and source files.') : undefined}><Trash2 size={14} />{text('删除', 'Delete')}</Button>
    </div>
    {running && <small className="kv-field-hint">{text('请先停止等待再删除', 'Stop waiting before deleting')}</small>}
  </div>
}

/** A creation opened from the history wall: canvas on the left, details on the right. Arrow keys step through the
 *  filtered wall and the canvas eases between the creations' ratios. */
export function MediaDetail({ job, zh, errorMessage, cancelling, statusLabel, onClose, onCancel, onPrev, onNext, position, actions }: {
  job: MediaJob; zh: boolean; errorMessage: string; cancelling: boolean; statusLabel: string
  onClose: () => void; onCancel: () => void; onPrev?: () => void; onNext?: () => void; position: string; actions: ResultActions
}) {
  const dialog = useRef<HTMLDialogElement>(null)
  const [measured, setMeasured] = useState<{ id: string; ratio: number } | null>(null)
  useEffect(() => {
    const node = dialog.current
    if (node && !node.open) node.showModal()
    return () => node?.close()
  }, [])
  const { text } = actions
  const canvas = jobCanvas(job, statusLabel, measured?.id === job.id ? measured.ratio : undefined, (ratio) => setMeasured({ id: job.id, ratio }))
  return <dialog ref={dialog} className="kv-modal kv-media-detail" aria-label={text('作品详情', 'Creation details')}
    onCancel={(e) => { e.preventDefault(); onClose() }} onClick={(e) => { if (e.target === e.currentTarget) onClose() }}
    onKeyDown={(e) => {
      if ((e.target as HTMLElement).closest('input, textarea, video, [role="listbox"]')) return
      if (e.key === 'ArrowLeft' && onPrev) { e.preventDefault(); onPrev() }
      if (e.key === 'ArrowRight' && onNext) { e.preventDefault(); onNext() }
    }}>
    <div className="kv-media-detail-stage">
      <MediaCanvas ratio={canvas.ratio} state={canvas.state}>{canvas.content}</MediaCanvas>
    </div>
    <aside className="kv-media-detail-side">
      <header className="kv-media-detail-header">
        {position && <span className="kv-media-detail-position">{position}</span>}
        <span className="kv-media-detail-spacer" />
        <IconButton size="sm" variant="ghost" label={text('上一个', 'Previous')} onClick={onPrev} disabled={!onPrev}><ChevronLeft size={16} /></IconButton>
        <IconButton size="sm" variant="ghost" label={text('下一个', 'Next')} onClick={onNext} disabled={!onNext}><ChevronRight size={16} /></IconButton>
        <IconButton size="sm" variant="ghost" label={text('关闭', 'Close')} onClick={onClose}><X size={15} /></IconButton>
      </header>
      <div key={job.id} className="kv-media-detail-body custom-scrollbar">
        {errorMessage && <p role="alert" className="kv-panel warn">{errorMessage}</p>}
        <JobDetails job={job} text={text} zh={zh} statusLabel={statusLabel} onCancel={onCancel} cancelling={cancelling} />
      </div>
      <footer className="kv-media-detail-actions"><MediaResultActions job={job} {...actions} /></footer>
    </aside>
  </dialog>
}
