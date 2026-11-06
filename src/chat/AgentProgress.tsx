import { subagentProgress } from '../api/subagents'

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
      {output && <AgentOutput content={output} zh={zh} />}
      {progress.error && <p role="alert" className="whitespace-pre-wrap break-words">{progress.error}</p>}
    </>}
  </div>
}

export function AgentOutput({ content, zh, lines = 12 }: { content: string; zh: boolean; lines?: number }) {
  const parts = content.split('\n')
  const long = parts.length > lines || content.length > 2000
  if (!long) return <pre className="whitespace-pre-wrap break-words font-sans">{content}</pre>
  const preview = parts.slice(0, lines).join('\n').slice(0, 2000)
  return <div><pre className="whitespace-pre-wrap break-words font-sans">{preview}…</pre><details>
    <summary className="cursor-pointer text-xs text-[var(--text-muted)]">{zh ? '展开完整输出' : 'Show full output'} ({content.length.toLocaleString()} {zh ? '字符' : 'characters'})</summary>
    <pre className="max-h-96 overflow-auto whitespace-pre-wrap break-words font-sans custom-scrollbar">{content}</pre>
  </details></div>
}
