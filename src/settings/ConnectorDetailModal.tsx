// 連接器詳情彈層：點選卡片開啟。
// 左欄總覽/賬戶/連結/開發者，右欄工具列表 + 逐工具允許/停用開關。
// 已連線才拉工具列表；未連線（可用）只顯示總覽/連結 + 連線按鈕。
// 複用 .kv-modal-backdrop / .kv-modal 風格（見 ProviderModelsPicker / index.css）。

import { useCallback, useEffect, useMemo, useState } from 'react'
import { Loader2, Search, Trash2, UserRound, X } from 'lucide-react'
import { api, type ChatMcpServer } from '../api/tauri'
import { i18n, type Lang } from './i18n'
import { Input } from './components'
import type { ConnectorCatalogEntry } from './connectorCatalog'
import { isToolAllowed, toggleTool } from './connectorToolToggle'

type ConnectorToolInfo = { name: string; description: string }

type Props = {
  lang: Lang
  entry?: ConnectorCatalogEntry
  /** 已連線時存在的 server；未連線為 null。 */
  server: ChatMcpServer | null
  /** 兜底展示名稱（自訂連接器無 catalog 項時）。 */
  fallbackName: string
  /** 兜底端點 URL（自訂連接器）。 */
  fallbackUrl?: string
  onClose: () => void
  /** 寫回某條 server（用於更新 enabledTools）。 */
  onUpdateServer: (server: ChatMcpServer) => void
  /** 在 modal 內斷開連線。 */
  onDisconnect: (serverId: string) => void
  /** 在 modal 內發起連線（token / oauth 由父級決定）。 */
  onConnect: () => void
  /** 連線按鈕是否處於忙碌態。 */
  connectBusy: boolean
  /** vault 類連接器：已儲存的本機路徑。 */
  vaultPath?: string
  /** vault 類連接器：斷開（清空路徑）。 */
  onDisconnectVault?: () => void
}

export function ConnectorDetailModal({
  lang,
  entry,
  server,
  fallbackName,
  fallbackUrl,
  onClose,
  onUpdateServer,
  onDisconnect,
  onConnect,
  connectBusy,
  vaultPath,
  onDisconnectVault,
}: Props) {
  const t = i18n[lang]
  const connected = !!server || !!vaultPath?.trim()
  const Name = server?.name ?? entry?.name ?? fallbackName
  const description = entry?.description[lang] ?? server?.url ?? fallbackUrl ?? ''

  const [tools, setTools] = useState<ConnectorToolInfo[] | null>(null)
  const [loadingTools, setLoadingTools] = useState(false)
  const [query, setQuery] = useState('')

  // Esc 關閉。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  // 已連線才拉工具列表。
  useEffect(() => {
    if (!server) {
      setTools(null)
      return
    }
    let cancelled = false
    setLoadingTools(true)
    api
      .chatMcpListToolDefs(server.id)
      .then((defs) => {
        if (!cancelled) setTools(defs)
      })
      .catch(() => {
        if (!cancelled) setTools([])
      })
      .finally(() => {
        if (!cancelled) setLoadingTools(false)
      })
    return () => {
      cancelled = true
    }
  }, [server])

  const allToolNames = useMemo(() => (tools ?? []).map((tool) => tool.name), [tools])

  const filteredTools = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!tools) return []
    if (!q) return tools
    return tools.filter(
      (tool) =>
        tool.name.toLowerCase().includes(q) || tool.description.toLowerCase().includes(q),
    )
  }, [tools, query])

  const setToolAllowed = useCallback(
    (toolName: string, allow: boolean) => {
      if (!server) return
      const next = toggleTool(allToolNames, server.enabledTools, toolName, allow)
      onUpdateServer({ ...server, enabledTools: next })
    },
    [allToolNames, onUpdateServer, server],
  )

  const openLink = useCallback((url?: string) => {
    if (url) void api.openExternal(url)
  }, [])

  return (
    <div
      className="kv-modal-backdrop"
      data-tauri-drag-region="false"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div
        className="kv-modal kv-connector-detail"
        role="dialog"
        aria-modal="true"
        data-tauri-drag-region="false"
        onMouseDown={(e) => e.stopPropagation()}
      >
        {/* 頭部 */}
        <div className="kv-connector-detail-header">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <div className="truncate text-sm font-medium">{Name}</div>
              <span
                className={
                  connected
                    ? 'kv-tag text-emerald-600 dark:text-emerald-400'
                    : 'kv-tag opacity-70'
                }
              >
                {connected ? t.connectorsDetailStatusConnected : t.connectorsDetailStatusAvailable}
              </span>
            </div>
            {description && (
              <div className="kv-row-desc mt-1 text-[12px]">{description}</div>
            )}
          </div>
          <button
            type="button"
            className="kv-icon-btn shrink-0"
            onClick={onClose}
            data-tauri-drag-region="false"
            aria-label={t.connectorsDetailClose}
          >
            <X size={14} />
          </button>
        </div>

        <div className="kv-connector-detail-body custom-scrollbar">
          {/* 左欄 */}
          <div className="kv-connector-detail-left">
            {entry?.overview && entry.overview[lang].length > 0 && (
              <section>
                <h4 className="kv-connector-detail-section-title">
                  {t.connectorsDetailOverview}
                </h4>
                <ul className="kv-connector-detail-overview">
                  {entry.overview[lang].map((point, i) => (
                    <li key={i}>{point}</li>
                  ))}
                </ul>
              </section>
            )}

            {vaultPath?.trim() && (
              <section>
                <h4 className="kv-connector-detail-section-title">
                  {t.connectorsVaultSelect}
                </h4>
                <div className="kv-row-desc truncate font-mono text-[11px]">{vaultPath}</div>
              </section>
            )}

            {server?.auth?.account && (
              <section>
                <h4 className="kv-connector-detail-section-title">
                  {t.connectorsDetailAccount}
                </h4>
                <div className="kv-connector-detail-account">
                  <UserRound size={13} className="opacity-60 shrink-0" />
                  <span className="truncate text-[12px]">{server.auth.account}</span>
                </div>
              </section>
            )}

            {(entry?.website || entry?.support) && (
              <section>
                <h4 className="kv-connector-detail-section-title">{t.connectorsDetailLinks}</h4>
                <div className="flex flex-col gap-1">
                  {entry?.website && (
                    <button
                      type="button"
                      className="kv-connector-detail-link"
                      onClick={() => openLink(entry.website)}
                      data-tauri-drag-region="false"
                    >
                      {t.connectorsDetailLinkWebsite}
                    </button>
                  )}
                  {entry?.support && (
                    <button
                      type="button"
                      className="kv-connector-detail-link"
                      onClick={() => openLink(entry.support)}
                      data-tauri-drag-region="false"
                    >
                      {t.connectorsDetailLinkSupport}
                    </button>
                  )}
                </div>
              </section>
            )}

            {entry?.developer && (
              <section>
                <h4 className="kv-connector-detail-section-title">
                  {t.connectorsDetailDeveloper}
                </h4>
                <div className="kv-row-desc text-[12px]">{entry.developer}</div>
              </section>
            )}

            {/* 連線 / 斷開操作 */}
            <div className="pt-1">
              {connected ? (
                <button
                  type="button"
                  className="kv-btn sm danger"
                  onClick={() => {
                    if (server) onDisconnect(server.id)
                    else onDisconnectVault?.()
                  }}
                  data-tauri-drag-region="false"
                >
                  <Trash2 size={10} />
                  {t.connectorsDisconnect}
                </button>
              ) : (
                <button
                  type="button"
                  className="kv-btn sm primary"
                  disabled={connectBusy}
                  onClick={onConnect}
                  data-tauri-drag-region="false"
                >
                  {connectBusy ? (
                    <>
                      <Loader2 size={10} className="animate-spin" />
                      {t.connectorsConnecting}
                    </>
                  ) : entry?.authKind === 'oauth' ? (
                    t.connectorsOauthAuthorize
                  ) : (
                    t.connectorsConnect
                  )}
                </button>
              )}
            </div>
          </div>

          {/* 右欄：工具（僅 MCP 已連線） */}
          {connected && server && (
            <div className="kv-connector-detail-right">
              <div className="kv-connector-detail-right-header">
                <h4 className="kv-connector-detail-section-title">{t.connectorsDetailTools}</h4>
                <div className="kv-connector-detail-search">
                  <Search size={14} className="opacity-60" />
                  <Input
                    value={query}
                    onChange={setQuery}
                    placeholder={t.connectorsDetailToolSearch}
                    mono={false}
                  />
                </div>
              </div>
              <div className="kv-connector-detail-tools custom-scrollbar">
                {loadingTools ? (
                  <div className="kv-row-desc flex items-center gap-2 py-2 text-[12px]">
                    <Loader2 size={12} className="animate-spin opacity-70" />
                    {t.connectorsDetailToolsLoading}
                  </div>
                ) : filteredTools.length === 0 ? (
                  <div className="kv-row-desc py-2 text-[12px]">
                    {t.connectorsDetailToolsEmpty}
                  </div>
                ) : (
                  filteredTools.map((tool) => {
                    const allowed = isToolAllowed(server!.enabledTools, tool.name)
                    return (
                      <div key={tool.name} className="kv-connector-detail-tool">
                        <div className="min-w-0 flex-1">
                          <div className="truncate text-[12px] font-medium">{tool.name}</div>
                          {tool.description && (
                            <div className="kv-row-desc line-clamp-2 text-[11px]">
                              {tool.description}
                            </div>
                          )}
                        </div>
                        <div className="kv-seg shrink-0">
                          <button
                            type="button"
                            className={allowed ? 'active' : ''}
                            onClick={() => setToolAllowed(tool.name, true)}
                            data-tauri-drag-region="false"
                          >
                            {t.connectorsDetailToolAllow}
                          </button>
                          <button
                            type="button"
                            className={allowed ? '' : 'active'}
                            onClick={() => setToolAllowed(tool.name, false)}
                            data-tauri-drag-region="false"
                          >
                            {t.connectorsDetailToolDisable}
                          </button>
                        </div>
                      </div>
                    )
                  })
                )}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
