import { useLang } from '../../components/i18n'
import './market.css'

type PluginSection = 'plugins' | 'skill' | 'mcp'

export function PluginCenterHeading({ value, onChange }: {
  value: PluginSection
  onChange: (value: PluginSection) => void
}) {
  const zh = useLang() === 'zh'
  return (
    <div className="flex flex-wrap items-center gap-3" data-tauri-drag-region="false">
      <h1 className="m-0 text-[26px] font-bold text-[var(--text)]">
        {zh ? '插件市场' : 'Plugin marketplace'}
      </h1>
      <nav className="kv-plugin-segments" aria-label={zh ? '插件类别' : 'Plugin categories'}>
        {([['plugins', zh ? '插件' : 'Plugins'], ['skill', 'Skill'], ['mcp', 'MCP']] as const).map(([id, label]) => (
          <button key={id} type="button" className="kv-plugin-segment"
            aria-current={value === id ? 'page' : undefined}
            onClick={() => onChange(id)}>
            {label}
          </button>
        ))}
      </nav>
    </div>
  )
}
