import { useEffect, useId, useRef, useState } from 'react'
import { Check, Layers, X } from 'lucide-react'
import { Button, IconButton } from '../components/Button'
import { useT } from '../components/i18n'
import { FieldBlock, Input, Select, TextArea } from '../settings/public/controls'
import type { ChatAssistant, ChatSet } from './types'

// ponytail: same palette as AssistantCenter so 集/助手 colors match; duplicated 6-element array, not worth sharing
const setColors = ['#6A8FBD', '#2f6ff0', '#4F9D7A', '#8A6FBD', '#B7791F', '#5E8C6A']

interface SetDialogProps {
  set?: ChatSet | null
  assistants: ChatAssistant[]
  saving?: boolean
  error?: string
  onSave: (
    name: string,
    systemPrompt: string,
    defaultAssistantId: string | null,
    color: string | null,
  ) => void
  onClose: () => void
}

/** 新建 / 编辑集：只有名称必填；颜色、系统提示词、默认助手都可留空。 */
export function SetDialog({
  set,
  assistants,
  saving = false,
  error = '',
  onSave,
  onClose,
}: SetDialogProps) {
  const t = useT()
  const [name, setName] = useState(set?.name ?? '')
  const [systemPrompt, setSystemPrompt] = useState(
    set?.system_prompt ?? set?.systemPrompt ?? '',
  )
  const [defaultAssistantId, setDefaultAssistantId] = useState(
    set?.default_assistant_id ?? set?.defaultAssistantId ?? '',
  )
  const [color, setColor] = useState<string | null>(set?.color ?? null)
  const dialogRef = useRef<HTMLDialogElement>(null)
  // 在对话框内按下、拖到遮罩上松开（例如选中文字）不算点遮罩。
  const pressedBackdrop = useRef(false)
  const titleId = useId()
  const nameId = useId()
  const promptId = useId()
  const title = set ? t.chatEditSet : t.chatNewSet

  const assistantOptions = [
    { value: '', label: t.chatSetDefaultAssistantNone },
    ...assistants
      .filter((a) => !a.archived || a.id === defaultAssistantId)
      .map((a) => ({ value: a.id, label: a.name })),
  ]

  useEffect(() => {
    const dialog = dialogRef.current
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null
    if (dialog && !dialog.open) dialog.showModal?.()
    const nameInput = dialog?.querySelector<HTMLInputElement>('input')
    nameInput?.focus()
    nameInput?.select()
    return () => {
      if (previous?.isConnected) previous.focus()
    }
  }, [])

  const canSubmit = Boolean(name.trim()) && !saving
  const submit = () => {
    if (!canSubmit) return
    onSave(name.trim(), systemPrompt.trim(), defaultAssistantId.trim() || null, color)
  }

  return (
    <dialog
      ref={dialogRef}
      className="kv kv-modal chat-motion-modal-in overflow-hidden"
      aria-labelledby={titleId}
      style={{ width: 'min(480px, calc(100vw - 32px))', maxWidth: 480, maxHeight: 'calc(100dvh - 48px)', padding: 0 }}
      data-tauri-drag-region="false"
      onCancel={(e) => {
        e.preventDefault()
        onClose()
      }}
      onMouseDown={(e) => {
        pressedBackdrop.current = e.target === e.currentTarget
      }}
      onClick={(e) => {
        const fromBackdrop = pressedBackdrop.current && e.target === e.currentTarget
        pressedBackdrop.current = false
        if (fromBackdrop) onClose()
      }}
    >
      <form
        className="flex max-h-[calc(100dvh-48px)] min-h-0 flex-col"
        onSubmit={(e) => {
          e.preventDefault()
          submit()
        }}
        onKeyDown={(e) => {
          // 多行提示词里回车是换行，Ctrl/⌘+Enter 直接提交。
          if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
            e.preventDefault()
            submit()
          }
        }}
      >
        <header className="flex shrink-0 items-start gap-3 px-5 pb-3 pt-5">
          {/* 图标即侧栏里的样子：选颜色时实时着色 */}
          <div
            className={`grid size-10 shrink-0 place-items-center rounded-xl transition-colors ${
              color ? '' : 'bg-neutral-900/[0.05] text-neutral-500'
            }`}
            style={color ? { backgroundColor: `${color}1f`, color } : undefined}
            aria-hidden="true"
          >
            <Layers size={19} strokeWidth={1.75} />
          </div>
          <div className="min-w-0 flex-1 pt-0.5">
            <h2 id={titleId} className="truncate text-[15px] font-semibold leading-5">
              {name.trim() || title}
            </h2>
            <p className="mt-0.5 text-[12px] leading-[18px] text-neutral-500">{t.chatSetDialogSubtitle}</p>
          </div>
          <IconButton label={t.cancel} variant="ghost" onClick={onClose}>
            <X size={16} strokeWidth={1.75} />
          </IconButton>
        </header>

        <div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto px-5 pb-2">
          <FieldBlock label={t.chatSetName} htmlFor={nameId}>
            <Input
              id={nameId}
              value={name}
              maxLength={80}
              onChange={setName}
              placeholder={t.chatSetNamePlaceholder}
            />
            <div className="mt-2.5 flex items-center gap-2" role="radiogroup" aria-label={t.chatSetColor}>
              <button
                type="button"
                role="radio"
                aria-checked={color === null}
                onClick={() => setColor(null)}
                className={`grid size-5 place-items-center rounded-full border transition-transform ${
                  color === null ? 'border-neutral-500' : 'border-neutral-300 hover:scale-110'
                }`}
                aria-label={t.chatSetNoColor}
                title={t.chatSetNoColor}
              >
                {color === null
                  ? <Check size={11} strokeWidth={2.5} className="text-neutral-600" />
                  : <span className="block h-px w-2.5 rotate-45 bg-neutral-400" />}
              </button>
              {setColors.map((c) => (
                <button
                  key={c}
                  type="button"
                  role="radio"
                  aria-checked={color === c}
                  onClick={() => setColor(c)}
                  className={`grid size-5 place-items-center rounded-full text-white transition-transform ${
                    color === c ? '' : 'hover:scale-110'
                  }`}
                  style={{ backgroundColor: c }}
                  aria-label={t.chatAssistantSelectColorNamed.replace('{name}', c)}
                >
                  {color === c && <Check size={11} strokeWidth={3} />}
                </button>
              ))}
            </div>
          </FieldBlock>

          <FieldBlock
            label={<>
              {t.chatSystemPrompt}
              <span className="ml-1.5 font-normal text-neutral-400">{t.chatSetOptional}</span>
            </>}
            description={t.chatSetSystemPromptHint}
            htmlFor={promptId}
          >
            <TextArea
              id={promptId}
              value={systemPrompt}
              onChange={setSystemPrompt}
              rows={5}
              maxRows={12}
              placeholder={t.chatSetSystemPromptPlaceholder}
            />
          </FieldBlock>

          <div className="flex items-center justify-between gap-4 py-2">
            <div className="min-w-0">
              <div className="kv-row-label">
                {t.chatSetDefaultAssistant}
                <span className="ml-1.5 font-normal text-neutral-400">{t.chatSetOptional}</span>
              </div>
              <p className="kv-row-desc">{t.chatSetDefaultAssistantHint}</p>
            </div>
            <Select
              className="w-[180px] shrink-0"
              value={defaultAssistantId}
              onChange={setDefaultAssistantId}
              options={assistantOptions}
              ariaLabel={t.chatSetDefaultAssistant}
            />
          </div>

          {error && <p className="kv-inline-error">{error}</p>}
        </div>

        <footer className="flex shrink-0 items-center justify-end gap-2 border-t border-[var(--color-border)] px-5 py-3">
          <Button size="sm" onClick={onClose}>
            {t.cancel}
          </Button>
          <Button type="submit" variant="primary" size="sm" disabled={!canSubmit}>
            {saving ? t.saving : set ? t.save : t.chatSetCreate}
          </Button>
        </footer>
      </form>
    </dialog>
  )
}
