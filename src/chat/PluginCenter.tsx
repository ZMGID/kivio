import { useEffect, useRef, useState, type ReactNode } from 'react'
import { i18n, LangContext, type Lang } from '../components/i18n'
import { PluginPackages } from './PluginPackages'

export type PluginCenterSection = 'plugins' | 'connectors'

interface PluginCenterTabsProps {
  section: PluginCenterSection
  onChange: (section: PluginCenterSection) => void
  lang: Lang
}

function PluginCenterTabs({ section, onChange, lang }: PluginCenterTabsProps) {
  const t = i18n[lang]
  const refs = useRef<Partial<Record<PluginCenterSection, HTMLButtonElement | null>>>({})
  const tabs = [
    ['plugins', t.tabPlugins],
    ['connectors', t.tabConnectors],
  ] as const

  return (
    <div className="mb-4 mt-2 flex w-fit items-center gap-1" role="tablist" aria-label={lang === 'zh' ? '插件与连接器' : 'Plugins and connectors'}>
      {tabs.map(([id, label], index) => (
        <button
          key={id}
          ref={node => { refs.current[id] = node }}
          id={`plugin-center-tab-${id}`}
          type="button"
          role="tab"
          aria-selected={section === id}
          aria-controls={`plugin-center-panel-${id}`}
          tabIndex={section === id ? 0 : -1}
          data-tauri-drag-region="false"
          onClick={() => onChange(id)}
          onKeyDown={event => {
            let next: PluginCenterSection
            if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
              const offset = event.key === 'ArrowRight' ? 1 : -1
              next = tabs[(index + offset + tabs.length) % tabs.length][0]
            } else if (event.key === 'Home') {
              next = tabs[0][0]
            } else if (event.key === 'End') {
              next = tabs[tabs.length - 1][0]
            } else {
              return
            }
            event.preventDefault()
            onChange(next)
            refs.current[next]?.focus()
          }}
          className={`rounded-lg px-3 py-1.5 text-[13px] transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-400 ${
            section === id
              ? 'bg-neutral-100 font-medium text-neutral-900 dark:bg-neutral-800 dark:text-neutral-50'
              : 'text-neutral-500 hover:bg-neutral-50 hover:text-neutral-800 dark:text-neutral-400 dark:hover:bg-neutral-900 dark:hover:text-neutral-200'
          }`}
        >
          {label}
        </button>
      ))}
    </div>
  )
}

interface PluginCenterProps {
  section: PluginCenterSection
  onSectionChange: (section: PluginCenterSection) => void
  lang: Lang
  connectors: ReactNode
}

export function PluginCenter({ section, onSectionChange, lang, connectors }: PluginCenterProps) {
  const [connectorsOpened, setConnectorsOpened] = useState(section === 'connectors')
  useEffect(() => {
    if (section === 'connectors') setConnectorsOpened(true)
  }, [section])

  return (
    <LangContext.Provider value={lang}>
      <PluginCenterTabs section={section} onChange={onSectionChange} lang={lang} />
      <div id="plugin-center-panel-plugins" role="tabpanel" aria-labelledby="plugin-center-tab-plugins" tabIndex={0} hidden={section !== 'plugins'}>
        <PluginPackages lang={lang} />
      </div>
      <div id="plugin-center-panel-connectors" role="tabpanel" aria-labelledby="plugin-center-tab-connectors" tabIndex={0} hidden={section !== 'connectors'}>
        {(connectorsOpened || section === 'connectors') && connectors}
      </div>
    </LangContext.Provider>
  )
}
