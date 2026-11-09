import { subagentProgress } from '../api/subagents'

export function AgentStatus({ state, zh }: { state: string; zh: boolean }) {
  if (state === 'idle' || state === 'parked') return null
  const label = (() => {
    switch (state) {
      case 'running': return zh ? '运行中' : 'Running'
      case 'completed': return zh ? '已完成' : 'Completed'
      case 'failed':
      case 'error': return zh ? '失败' : 'Failed'
      case 'aborted':
      case 'cancelled': return zh ? '已中断' : 'Cancelled'
      default: return zh ? '等待运行' : 'Queued'
    }
  })()
  return <span className="kv-agent-status" data-state={state}><span aria-hidden="true" />{label}</span>
}

export function AgentProgress({ value, expanded = true, zh }: { value: unknown; expanded?: boolean; zh: boolean }) {
  const progress = subagentProgress(value)
  if (!progress) return null
  const metrics = [
    progress.requests != null && `${progress.requests} ${zh ? '请求' : 'requests'}`,
    progress.toolCount != null && `${progress.toolCount} ${zh ? '工具' : 'tools'}`,
    progress.tokens != null && `${progress.tokens.toLocaleString()} ${zh ? '累计 token' : 'total tokens'}`,
    progress.contextTokens != null && `${zh ? '上下文' : 'context'} ${progress.contextTokens.toLocaleString()}${progress.contextWindow ? ` / ${progress.contextWindow.toLocaleString()}` : ''}`,
    progress.cost != null && `$${progress.cost.toFixed(4)}`,
    progress.durationMs != null && `${(progress.durationMs / 1000).toFixed(1)}s`,
    progress.completionPercent != null && `${zh ? '估计完成' : 'estimated completion'} ${progress.completionPercent}%`,
  ].filter(Boolean)
  const output = progress.preview || progress.recentOutput?.join('\n') || ''
  return <div className="min-w-0 space-y-1">
    {metrics.length > 0 && <p className="break-words text-xs text-[var(--text-muted)]">{metrics.join(' · ')}</p>}
    {progress.stage && <p className="break-words">{progress.stage}</p>}
    {(progress.currentToolIntent || progress.currentTool) && <p className="break-words font-mono text-xs text-[var(--text-muted)]" title={progress.currentToolArgs ?? undefined}>{progress.currentToolIntent || progress.currentTool}{progress.currentToolStartMs != null ? ` · ${Math.max(0, (Date.now() - progress.currentToolStartMs) / 1000).toFixed(1)}s` : ''}</p>}
    {expanded && <>
      {progress.recentTools?.length ? progress.recentTools.map((tool, index) => <p key={index} title={tool.args} className="break-words font-mono text-xs text-[var(--text-muted)]">{tool.intent || tool.tool}{tool.isError ? zh ? ' · 失败' : ' · Failed' : ''}</p>)
        : progress.steps?.map((step, index) => <p key={index} className="break-words font-mono text-xs text-[var(--text-muted)]">{step}</p>)}
      {output && <AgentOutput content={output} />}
      {progress.error && <p role="alert" className="whitespace-pre-wrap break-words">{progress.error}</p>}
    </>}
  </div>
}

export function AgentOutput({ content }: { content: string }) {
  const long = content.length > 2000 || content.split('\n').length > 12
  return <pre tabIndex={long ? 0 : undefined} className={`whitespace-pre-wrap [overflow-wrap:anywhere] font-sans${long ? ' max-h-60 overflow-auto custom-scrollbar' : ''}`}>{content}</pre>
}
