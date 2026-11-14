import { providerAuthenticationReady } from '../api/tauri'
import { useMemo, useRef } from 'react'
import { GripVertical, Pencil, Trash2 } from 'lucide-react'
import type { ModelProvider } from '../api/tauri'
import { IconButton } from '../components/Button'
import { ProviderIcon } from '../components/ModelIcon'
import type { I18n, Lang } from '../components/i18n'
import { usePointerReorder } from '../utils/pointerReorder'
import { Toggle } from './components'
import { isProviderEnabled } from './utils'

/** 卡片之间的间距（px）；拖拽落点按「行高 + 间距」换算，必须与 CSS 的 gap 一致。 */
const CARD_GAP = 8

type ProviderSortableListProps = {
  /** 当前要显示的供应商（可能已被搜索过滤）。 */
  providers: ModelProvider[]
  /** 列表被过滤时不允许拖拽：看不到的项无法确定落点。 */
  sortable: boolean
  t: I18n
  lang: Lang
  /** provider id → 手选的图标 key；缺省时按名字/域名自动匹配。 */
  icons?: Record<string, string>
  onOpen: (id: string) => void
  onToggleEnabled: (id: string, enabled: boolean) => void
  onRequestDelete: (id: string) => void
  onReorder: (fromId: string, toId: string) => void
}

/** 「已添加」的供应商卡片列表：整张卡片点开详情，右侧是启用开关、编辑和删除，左侧手柄拖拽排序。 */
export function ProviderSortableList({
  providers,
  sortable,
  t,
  lang,
  icons,
  onOpen,
  onToggleEnabled,
  onRequestDelete,
  onReorder,
}: ProviderSortableListProps) {
  const zh = lang === 'zh'
  const listRef = useRef<HTMLDivElement>(null)
  const ids = useMemo(() => providers.map((p) => p.id), [providers])
  const { draggingId, startDrag, itemTransform } = usePointerReorder({
    ids,
    listRef,
    itemSelector: '.kv-provider-card',
    rowGap: CARD_GAP,
    onReorder,
  })

  const dragLabel = zh ? '拖动调整顺序' : 'Drag to reorder'
  const editLabel = zh ? '编辑' : 'Edit'

  return (
    <div ref={listRef} className={`kv-provider-cards${draggingId ? ' is-sorting' : ''}`}>
      {providers.map((provider, index) => {
        const enabled = isProviderEnabled(provider)
        const configured = providerAuthenticationReady(provider)
        const name = provider.name || t.providerName
        const transform = itemTransform(index)
        const modelCount = provider.enabledModels.length

        return (
          <div
            key={provider.id}
            className={`kv-provider-card${enabled ? '' : ' is-off'}${draggingId === provider.id ? ' is-dragging' : ''}`}
            style={transform ? { transform } : undefined}
            data-tauri-drag-region="false"
            role="button"
            tabIndex={0}
            aria-label={name}
            onClick={() => onOpen(provider.id)}
            onKeyDown={(e) => {
              if (e.target !== e.currentTarget) return
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault()
                onOpen(provider.id)
              }
            }}
          >
            {sortable && (
              <button
                type="button"
                className="kv-provider-card-grip"
                aria-label={dragLabel}
                title={dragLabel}
                onPointerDown={(e) => startDrag(e, provider.id, index)}
                onClick={(e) => e.stopPropagation()}
                data-tauri-drag-region="false"
              >
                <GripVertical size={14} strokeWidth={2} />
              </button>
            )}
            <span className="kv-provider-card-icon">
              <ProviderIcon
                name={provider.name}
                baseUrl={provider.baseUrl}
                iconKey={icons?.[provider.id]}
                size={20}
              />
            </span>
            <span className="kv-provider-card-text">
              <span className="kv-provider-card-name">{name}</span>
              <span className="kv-provider-card-url">
                {provider.baseUrl || (zh ? '未填写 API 地址' : 'No API endpoint')}
              </span>
            </span>
            <span className="kv-provider-card-tags">
              {enabled && !configured && <span className="kv-tag warn">{t.permissionMissing}</span>}
              <span className="kv-tag tabular-nums">
                {zh ? `${modelCount} 个模型` : `${modelCount} model${modelCount === 1 ? '' : 's'}`}
              </span>
            </span>
            {/* 卡片本身可点；这一组控件各管各的，不该顺带打开详情。 */}
            <span
              className="kv-provider-card-actions"
              onClick={(e) => e.stopPropagation()}
              onKeyDown={(e) => e.stopPropagation()}
            >
              <Toggle
                checked={enabled}
                onChange={(next) => onToggleEnabled(provider.id, next)}
                ariaLabel={zh ? `启用 ${name}` : `Enable ${name}`}
              />
              <IconButton
                size="sm"
                label={editLabel}
                title={editLabel}
                onClick={() => onOpen(provider.id)}
                data-tauri-drag-region="false"
              >
                <Pencil size={13} />
              </IconButton>
              <IconButton
                variant="danger"
                size="sm"
                label={t.deleteProvider}
                title={t.deleteProvider}
                onClick={() => onRequestDelete(provider.id)}
                data-tauri-drag-region="false"
              >
                <Trash2 size={13} />
              </IconButton>
            </span>
          </div>
        )
      })}
    </div>
  )
}
