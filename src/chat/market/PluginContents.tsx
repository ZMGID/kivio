import { useEffect, useState, type ReactNode } from 'react'
import { Anchor, Bot, ExternalLink, Loader2, Server, Terminal, WandSparkles } from 'lucide-react'
import { packageApi, type PluginDetails } from '../../api/pluginPackages'
import { marketplaceApi } from '../../api/market'
import { api } from '../../api/tauri'
import { Button } from '../../components/Button'
import { useLang } from '../../components/i18n'

/** Owns a detail request; navigation/unmount isolates late results from the next plugin. */
export function PluginContents({ packageId, marketplaceId, plugin, version, information, children }: {
  packageId?: string; marketplaceId?: string; plugin?: string; version: string | null
  information: ReactNode; children?: ReactNode
}) {
  const zh = useLang() === 'zh'
  const text = (cn: string, en: string) => zh ? cn : en
  const [attempt, setAttempt] = useState(0)
  const [state, setState] = useState<{ details?: PluginDetails; error?: string }>({})
  useEffect(() => {
    let disposed = false
    setState({})
    const request = packageId ? packageApi.describe(packageId) : marketplaceApi.describe(marketplaceId!, plugin!)
    void request.then(details => { if (!disposed) setState({ details }) }, error => {
      if (!disposed) setState({ error: String(error) })
    })
    return () => { disposed = true }
  }, [packageId, marketplaceId, plugin, attempt])
  const kinds = {
    mcp: { label: text('MCP 服务器', 'MCP servers'), Icon: Server },
    skills: { label: text('技能', 'Skills'), Icon: WandSparkles },
    commands: { label: text('命令', 'Commands'), Icon: Terminal },
    agents: { label: text('智能体', 'Agents'), Icon: Bot },
    hooks: { label: 'Hooks', Icon: Anchor },
  }
  const details = state.details
  const homepage = details?.homepage && /^https?:\/\//i.test(details.homepage) ? details.homepage : null
  return <>
    {!details && <section className="kv-market-block" aria-live="polite">
      {state.error ? <>
        <p className="kv-market-error">{text('无法读取插件内容：', 'Could not read plugin contents: ')}{state.error}</p>
        <Button size="sm" onClick={() => setAttempt(value => value + 1)}>{text('重试', 'Retry')}</Button>
      </> : <p className="kv-market-detail-loading"><Loader2 size={15} className="animate-spin" />{text('正在读取插件内容…', 'Reading plugin contents…')}</p>}
    </section>}
    {details?.groups.map(group => {
      const { Icon, label } = kinds[group.kind]
      return <section className="kv-market-block" key={group.kind}>
        <h2>{label} <span className="kv-market-count">{group.items.length}</span></h2>
        {group.items.map(item => <div className="kv-market-capability" key={item.name}>
          <span className="kv-market-capability-icon"><Icon size={16} /></span>
          <div className="kv-market-component"><strong>{item.name}</strong>{item.description && <span title={item.description}>{item.description}</span>}</div>
        </div>)}
      </section>
    })}
    {details && !details.groups.length && !details.diagnostics.length && <p className="kv-market-muted">{text('这个插件未声明可展示的组件。', 'This plugin declares no displayable components.')}</p>}
    {!!details?.diagnostics.length && <section className="kv-market-block">
      <h2>{text('部分内容未能读取', 'Some contents could not be read')}</h2>
      {details.diagnostics.map((message, index) => <p className="kv-market-warning" key={index}>{message}</p>)}
    </section>}
    <section className="kv-market-block"><h2>{text('信息', 'Information')}</h2>
      <dl className="kv-market-info">
        {details?.author && <><dt>{text('开发者', 'Developer')}</dt><dd>{details.author}</dd></>}
        <dt>{text('版本', 'Version')}</dt><dd>{details?.version ?? version ?? '—'}</dd>
        {details?.license && <><dt>{text('许可证', 'License')}</dt><dd>{details.license}</dd></>}
        {homepage && <><dt>{text('网站', 'Website')}</dt><dd><Button variant="ghost" size="sm" onClick={() => void api.openExternal(homepage)}>{homepage}<ExternalLink size={13} /></Button></dd></>}
        {information}
      </dl>
      {children}
    </section>
  </>
}
