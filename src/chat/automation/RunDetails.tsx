import { useEffect, useState } from 'react'
import { Button } from '../../components/Button'
import { useLang, useT } from '../../components/i18n'
import type { Automation, AutomationRun, NodeOutput } from '../../api/automationContracts'
import { automationApi } from './api'

/** Historical snapshots stay separate from the live run used by execution controls. */
export function RunDetails({ automation, runId, onClose, onLocate }: {
  automation: Automation; runId: string; onClose: () => void; onLocate: (nodeId: string) => void
}) {
  const english = useLang() === 'en'
  const t = useT()
  const [record, setRecord] = useState<AutomationRun | null>(null)
  const [error, setError] = useState('')
  const [attempt, setAttempt] = useState(0)
  useEffect(() => {
    let disposed = false
    setRecord(null)
    setError('')
    void automationApi.getRun(automation.id, runId).then((run) => {
      if (!disposed) setRecord(run)
    }).catch((err: unknown) => {
      if (!disposed) setError(err instanceof Error ? err.message : String(err))
    })
    return () => { disposed = true }
  }, [automation.id, runId, attempt])
  const status = (value: string) => ({
    success: t.chatAutomationStatusSuccess, error: t.chatAutomationStatusError,
    running: t.chatAutomationStatusRunning, cancelled: t.chatAutomationCancelled,
    skipped: english ? 'Skipped' : '已跳过',
  })[value] ?? value
  const duration = record?.finishedAt
    ? Math.max(0, (Date.parse(record.finishedAt) - Date.parse(record.startedAt)) / 1000) : null
  return <section className="kv-workbench" aria-label={english ? 'Run details' : '运行详情'}>
    <div className="flex items-center justify-between gap-2 p-3">
      <h2>{english ? 'Run details' : '运行详情'}</h2>
      <Button size="sm" variant="ghost" onClick={onClose}>{english ? 'Back to editing' : '返回编辑'}</Button>
    </div>
    <div className="kv-workbench-content kv-workbench-data custom-scrollbar">
      {error ? <><p role="alert">{error}</p><Button size="sm" onClick={() => setAttempt((n) => n + 1)}>{english ? 'Retry' : '重试'}</Button></>
        : !record ? <p role="status">{t.chatLoading}</p> : <>
          <p>{status(record.status)} · {new Date(record.startedAt).toLocaleString()}
            {duration !== null && Number.isFinite(duration) ? ` · ${duration}s` : ''}</p>
          <p>{english ? 'Recorded data from this execution. Node names follow the current canvas.' : '以下是这次执行保存的数据，节点名称以当前画布为准。'}</p>
          {record.error && <p role="alert">{record.error}</p>}
          {record.nodes.length === 0 && <p>{english ? 'No node records.' : '没有节点记录。'}</p>}
          {record.nodes.map((node, index) => {
            const current = automation.nodes.find((item) => item.id === node.nodeId)
            return <details key={node.nodeId} open={node.status === 'error'}>
              <summary>{index + 1}. {current?.data.label || node.nodeType} · {status(node.status)}</summary>
              {current ? <Button size="sm" variant="ghost" onClick={() => onLocate(node.nodeId)}>{english ? 'Locate on canvas' : '定位到画布'}</Button>
                : <p>{english ? 'This node has been deleted.' : '此节点已从画布删除。'}</p>}
              {node.error && <p role="alert">{node.error}</p>}
              <h3>{english ? 'Input' : '输入'}</h3>
              <Snapshot value={node.input} english={english} />
              <h3>{english ? 'Output' : '输出'}</h3>
              <Snapshot value={node.result} fallback={node.output} english={english} />
            </details>
          })}
        </>}
    </div>
  </section>
}

function Snapshot({ value, fallback, english }: { value?: NodeOutput | null; fallback?: string | null; english: boolean }) {
  return value ? <pre className="whitespace-pre-wrap break-words">{JSON.stringify({ text: value.text, json: value.json }, null, 2)}</pre>
    : <><p>{english ? 'Full snapshot unavailable (not recorded or exceeded the storage limit).' : '完整数据不可用（未记录或超过保存上限）。'}</p>
      {fallback && <pre className="whitespace-pre-wrap break-words">{fallback}</pre>}</>
}
