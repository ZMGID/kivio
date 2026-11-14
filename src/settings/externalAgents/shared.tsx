import { useState, type ReactNode } from 'react'
import { AgentIcon } from '../../components/AgentIcon'
import { Input } from '../components'

/**
 * 失焦或回车才提交的输入框，Esc 还原。
 *
 * 设置草稿是自动保存的：直接绑定的话，路径打到一半就会被存下并拿去重新探测 CLI，
 * 环境变量名打到一半也会被注入子进程。
 */
export function DraftInput({
  value,
  onCommit,
  ...props
}: {
  value: string
  onCommit: (value: string) => void
} & Omit<React.ComponentProps<typeof Input>, 'value' | 'onChange'>) {
  const [draft, setDraft] = useState(value)
  const [seen, setSeen] = useState(value)
  if (value !== seen) {
    setSeen(value)
    setDraft(value)
  }
  return (
    <Input
      {...props}
      value={draft}
      onChange={setDraft}
      onBlur={(event) => {
        if (draft !== value) onCommit(draft)
        props.onBlur?.(event)
      }}
      onKeyDown={(event) => {
        if (event.nativeEvent.isComposing) return
        if (event.key === 'Enter') event.currentTarget.blur()
        else if (event.key === 'Escape' && draft !== value) {
          event.stopPropagation()
          setDraft(value)
        }
        props.onKeyDown?.(event)
      }}
    />
  )
}

export function IconBox({ id, size }: { id: string; size: number }) {
  return (
    <span className="kv-cli-iconbox" style={{ width: size, height: size }}>
      <AgentIcon id={id} size={Math.round(size * 0.6)} />
    </span>
  )
}

/** 详情里的一个分区：标题与动作同一行，说明另起一行，免得长说明把按钮挤换行。 */
export function Section({
  title,
  desc,
  actions,
  children,
}: {
  title: string
  desc?: ReactNode
  actions?: ReactNode
  children: ReactNode
}) {
  return (
    <section className="kv-cli-section">
      <div className="kv-cli-section-head">
        <h3 className="kv-cli-section-title">{title}</h3>
        {actions && <div className="kv-cli-section-actions">{actions}</div>}
      </div>
      {desc && <p className="kv-cli-section-desc">{desc}</p>}
      {children}
    </section>
  )
}

/** 卡片里的一行：说明在左，控件在右。 */
export function Row({
  label,
  desc,
  lead,
  children,
}: {
  label: ReactNode
  desc?: ReactNode
  lead?: ReactNode
  children?: ReactNode
}) {
  return (
    <div className="kv-row">
      {lead}
      <div className="kv-row-text">
        <div className="kv-row-label">{label}</div>
        {desc && <p className="kv-row-desc">{desc}</p>}
      </div>
      {children && <div className="kv-row-control">{children}</div>}
    </div>
  )
}
