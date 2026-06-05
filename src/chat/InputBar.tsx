import { useCallback, useEffect, useRef, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { ArrowUp, Plus, SlidersHorizontal, Square } from 'lucide-react'
import { ChatAttachments } from './ChatAttachments'
import type { ChatToolDefinition } from '../api/tauri'
import type { PendingAttachment } from './types'

const IMAGE_EXTENSIONS = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'tiff', 'tif', 'heic', 'heif']
const DOCUMENT_EXTENSIONS = ['pdf', 'doc', 'docx', 'xls', 'xlsx', 'xlsm', 'csv', 'tsv']
const SUPPORTED_ATTACHMENT_EXTENSIONS = [...IMAGE_EXTENSIONS, ...DOCUMENT_EXTENSIONS]
const SUPPORTED_ATTACHMENT_LABEL = '图片、PDF、Word、Excel/CSV/TSV'
const isTauriRuntime = () => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

function shouldComposerAutoFocus(activeElement: Element | null): boolean {
  if (!activeElement || activeElement === document.body || activeElement === document.documentElement) {
    return true
  }
  if (activeElement instanceof HTMLTextAreaElement || activeElement instanceof HTMLInputElement) {
    return false
  }
  return activeElement.closest('[data-chat-composer="true"]') !== null
}

function isExternalMcpTool(tool: ChatToolDefinition): boolean {
  return tool.source !== 'skill' && tool.source !== 'native'
}

interface InputBarProps {
  onSend: (content: string, attachments: PendingAttachment[]) => void
  disabled?: boolean
  onCancel?: () => void
  cancelVisible?: boolean
  cancelling?: boolean
  onOpenSettings?: () => void
  enabledTools?: ChatToolDefinition[]
  toolsDisabledReason?: string
  toolStatusHint?: string
  sendDisabledReason?: string
  enabledSkills?: { id: string; name: string }[]
  onOpenSkillSettings?: () => void
  autoFocus?: boolean
  /** footer：贴底（有消息时）；inline：嵌入居中区域（空对话欢迎页） */
  layout?: 'footer' | 'inline'
}

export function InputBar({
  onSend,
  disabled,
  onCancel,
  cancelVisible,
  cancelling,
  onOpenSettings,
  enabledTools = [],
  toolsDisabledReason,
  toolStatusHint,
  sendDisabledReason,
  enabledSkills = [],
  onOpenSkillSettings,
  autoFocus,
  layout = 'footer',
}: InputBarProps) {
  const [input, setInput] = useState('')
  const [attachments, setAttachments] = useState<PendingAttachment[]>([])
  const [attachmentError, setAttachmentError] = useState('')
  const [dragActive, setDragActive] = useState(false)
  const [toolPanelOpen, setToolPanelOpen] = useState(false)
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  const attachmentsFromPaths = useCallback(
    (paths: string[]) =>
      paths.map((path) => {
        const normalized = path.replace(/\\/g, '/')
        const name = normalized.split('/').filter(Boolean).pop() || '附件'
        const ext = name.split('.').pop()?.toLowerCase() ?? ''
        const type: PendingAttachment['type'] = IMAGE_EXTENSIONS.includes(ext) ? 'image' : 'file'
        return {
          id: `pending-att-${crypto.randomUUID()}`,
          type,
          name,
          path,
        }
      }),
    [],
  )

  const addAttachments = useCallback(
    (next: PendingAttachment[], options?: { imagesOnly?: boolean }) => {
      const filtered = options?.imagesOnly
        ? next.filter((attachment) => attachment.type === 'image')
        : next.filter((attachment) => {
          const ext = attachment.name.split('.').pop()?.toLowerCase() ?? ''
          return SUPPORTED_ATTACHMENT_EXTENSIONS.includes(ext)
        })
      if (filtered.length === 0) {
        setAttachmentError(options?.imagesOnly ? '请拖入图片文件' : `仅支持${SUPPORTED_ATTACHMENT_LABEL}`)
        return
      }

      setAttachments((prev) => {
        const existing = new Set(prev.map((attachment) => attachment.path))
        const dedupedNext = filtered.filter((attachment) => {
          if (existing.has(attachment.path)) return false
          existing.add(attachment.path)
          return true
        })
        if (dedupedNext.length === 0) {
          setAttachmentError('附件已添加')
          return prev
        }
        setAttachmentError('')
        return [...prev, ...dedupedNext]
      })
      textareaRef.current?.focus()
    },
    [],
  )

  const handleSend = () => {
    const trimmed = input.trim()
    if ((!trimmed && attachments.length === 0) || disabled || sendDisabledReason) return
    onSend(trimmed, attachments)
    setInput('')
    setAttachments([])
    setAttachmentError('')
    setToolPanelOpen(false)
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto'
    }
  }

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      handleSend()
    }
  }

  const handleInput = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setInput(e.target.value)
    const el = e.target
    el.style.height = 'auto'
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`
  }

  const handleAddAttachment = async () => {
    if (disabled) return
    setToolPanelOpen(false)
    setAttachmentError('')
    try {
      const selected = await open({
        multiple: true,
        directory: false,
        filters: [
          {
            name: '支持的附件',
            extensions: SUPPORTED_ATTACHMENT_EXTENSIONS,
          },
          {
            name: '图片',
            extensions: IMAGE_EXTENSIONS,
          },
          {
            name: '文档与表格',
            extensions: DOCUMENT_EXTENSIONS,
          },
        ],
      })
      const paths = Array.isArray(selected) ? selected : selected ? [selected] : []
      if (paths.length === 0) return

      addAttachments(attachmentsFromPaths(paths))
    } catch (err) {
      console.error('Failed to add chat attachment:', err)
      setAttachmentError(
        typeof err === 'string' ? err : err instanceof Error ? err.message : '添加附件失败',
      )
    }
  }

  const removeAttachment = (id: string) => {
    setAttachments((prev) => prev.filter((attachment) => attachment.id !== id))
    setAttachmentError('')
  }

  useEffect(() => {
    if (!autoFocus || disabled) return
    requestAnimationFrame(() => {
      if (shouldComposerAutoFocus(document.activeElement)) {
        textareaRef.current?.focus({ preventScroll: true })
      }
    })
  }, [autoFocus, disabled])

  useEffect(() => {
    if (!autoFocus || !isTauriRuntime()) return
    let cancelled = false
    let unlisten: (() => void) | undefined

    getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (!focused || cancelled) return
      requestAnimationFrame(() => {
        if (!cancelled && !disabled && shouldComposerAutoFocus(document.activeElement)) {
          textareaRef.current?.focus({ preventScroll: true })
        }
      })
    }).then((handler) => {
      if (cancelled) {
        handler()
      } else {
        unlisten = handler
      }
    }).catch((err) => {
      console.error('Failed to listen for chat input focus changes:', err)
    })

    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [autoFocus, disabled])

  useEffect(() => {
    if (!toolPanelOpen) return
    const handleEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setToolPanelOpen(false)
      }
    }
    window.addEventListener('keydown', handleEscape)
    return () => window.removeEventListener('keydown', handleEscape)
  }, [toolPanelOpen])

  useEffect(() => {
    if (disabled) setToolPanelOpen(false)
  }, [disabled])

  useEffect(() => {
    if (!isTauriRuntime()) return
    let cancelled = false
    let unlisten: (() => void) | undefined

    getCurrentWebview().onDragDropEvent((event) => {
      if (cancelled || disabled) return

      if (event.payload.type === 'enter' || event.payload.type === 'over') {
        setDragActive(true)
        setAttachmentError('')
        return
      }

      if (event.payload.type === 'leave') {
        setDragActive(false)
        return
      }

      if (event.payload.type === 'drop') {
        setDragActive(false)
        addAttachments(attachmentsFromPaths(event.payload.paths))
      }
    }).then((handler) => {
      if (cancelled) {
        handler()
      } else {
        unlisten = handler
      }
    }).catch((err) => {
      console.error('Failed to listen for chat attachment drops:', err)
    })

    return () => {
      cancelled = true
      setDragActive(false)
      unlisten?.()
    }
  }, [addAttachments, attachmentsFromPaths, disabled])

  const canSend = (Boolean(input.trim()) || attachments.length > 0) && !disabled && !sendDisabledReason

  const wrapperClass =
    layout === 'inline'
      ? 'w-full'
      : 'chat-composer-footer shrink-0 px-6 pb-8 pt-2'

  const innerClass = layout === 'inline' ? 'w-full' : 'mx-auto w-full max-w-3xl'
  const externalMcpTools = enabledTools.filter(isExternalMcpTool)
  const hasToolProblem = Boolean(toolsDisabledReason || toolStatusHint || sendDisabledReason)
  const showMcpSection = externalMcpTools.length > 0 || Boolean(toolsDisabledReason)
  const mcpStatusLine = toolsDisabledReason
    || (externalMcpTools.length > 0 ? `MCP ${externalMcpTools.length}` : '')

  return (
    <div className={wrapperClass}>
      <div className={`relative ${innerClass}`}>
        {toolPanelOpen && (
          <>
            <div className="fixed inset-0 z-30" onClick={() => setToolPanelOpen(false)} aria-hidden />
            <div
              className="chat-motion-popover absolute bottom-full left-10 z-40 mb-2 w-[min(272px,calc(100vw-32px))] overflow-hidden rounded-xl border border-neutral-200/90 bg-white shadow-[0_10px_28px_rgba(0,0,0,0.14)] dark:border-neutral-700 dark:bg-neutral-900"
              style={{ ['--chat-popover-origin' as string]: 'bottom left' }}
              data-tauri-drag-region="false"
            >
              <div className="space-y-1.5 px-3 py-2">
                <div className="flex items-center justify-between gap-2">
                  <span className="text-[12px] font-semibold text-neutral-800 dark:text-neutral-100">Skill</span>
                  {onOpenSkillSettings && (
                    <button
                      type="button"
                      onClick={() => {
                        setToolPanelOpen(false)
                        onOpenSkillSettings()
                      }}
                      className="rounded-md px-1.5 py-0.5 text-[11px] text-neutral-500 transition-colors hover:bg-neutral-100 hover:text-neutral-800 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
                    >
                      管理
                    </button>
                  )}
                </div>
                <div className="text-[11px] leading-4 text-neutral-600 dark:text-neutral-300">
                  <span className="text-neutral-500 dark:text-neutral-400">
                    已启用 {enabledSkills.length} 个
                  </span>
                  {enabledSkills.length > 0 && (
                    <>
                      <span className="text-neutral-300 dark:text-neutral-600"> · </span>
                      <span className="text-neutral-700 dark:text-neutral-200">
                        {enabledSkills.map((skill) => skill.name).join('、')}
                      </span>
                    </>
                  )}
                </div>

                {showMcpSection && mcpStatusLine && (
                  <div className="border-t border-neutral-200/80 pt-1.5 text-[11px] text-neutral-500 dark:border-neutral-800 dark:text-neutral-400">
                    {mcpStatusLine}
                  </div>
                )}

                {(sendDisabledReason || toolStatusHint) && (
                  <p className="rounded-md bg-amber-50 px-2 py-1 text-[11px] leading-4 text-amber-700 dark:bg-amber-400/10 dark:text-amber-200">
                    {sendDisabledReason || toolStatusHint}
                  </p>
                )}
              </div>
            </div>
          </>
        )}
        <div
          data-chat-composer="true"
          className={`chat-composer-shell rounded-[28px] border bg-white px-3 py-2.5 transition-[box-shadow,border-color] duration-200 dark:bg-neutral-900 ${
            dragActive
              ? 'border-[#e8a090] shadow-[0_2px_12px_rgba(0,0,0,0.06)] ring-2 ring-[#e8a090]/25 dark:border-[#e8a090] dark:shadow-none'
              : 'border-neutral-200/80 shadow-[0_1px_2px_rgba(0,0,0,0.04),0_12px_32px_-14px_rgba(0,0,0,0.14)] focus-within:border-neutral-300 focus-within:shadow-[0_1px_3px_rgba(0,0,0,0.05),0_18px_44px_-16px_rgba(0,0,0,0.20)] dark:border-neutral-700 dark:shadow-none dark:focus-within:border-neutral-600'
          }`}
        >
          {dragActive && (
            <div className="chat-motion-fade-up mb-2 rounded-2xl border border-dashed border-[#e8a090]/70 bg-[#e8a090]/10 px-3 py-2 text-center text-[13px] font-medium text-[#a35f51] dark:text-[#f1b4a7]">
              松开即可添加附件
            </div>
          )}
          {attachments.length > 0 && (
            <div className="chat-motion-fade-up mb-2 px-1">
              <ChatAttachments
                attachments={attachments}
                variant="composer"
                onRemove={disabled ? undefined : removeAttachment}
              />
            </div>
          )}
          {attachmentError && (
            <div className="chat-motion-fade-up mb-2 px-1 text-[12px] text-red-500 dark:text-red-400">
              {attachmentError}
            </div>
          )}
          {(sendDisabledReason || toolStatusHint) && !attachmentError && (
            <div className="chat-motion-fade-up mb-2 px-1 text-[12px] text-amber-600 dark:text-amber-300">
              {sendDisabledReason || toolStatusHint}
            </div>
          )}
          <div className="flex items-end gap-2">
            <button
              type="button"
              onClick={() => void handleAddAttachment()}
              disabled={disabled}
              tabIndex={-1}
              className="mb-0.5 shrink-0 rounded-full p-2 text-neutral-500 transition-colors hover:bg-neutral-100 disabled:opacity-40 dark:hover:bg-neutral-800"
              title="添加附件"
              aria-label="添加附件"
            >
              <Plus size={20} strokeWidth={1.75} />
            </button>

            {onOpenSettings && (
              <button
                type="button"
                onClick={() => {
                  setToolPanelOpen((open) => !open)
                }}
                disabled={disabled}
                tabIndex={-1}
                className={`mb-0.5 shrink-0 rounded-full p-2 transition-colors disabled:opacity-40 ${
                  toolPanelOpen || hasToolProblem || enabledSkills.length > 0 || externalMcpTools.length > 0
                    ? 'bg-neutral-100 text-neutral-800 dark:bg-neutral-800 dark:text-neutral-100'
                    : 'text-neutral-500 hover:bg-neutral-100 dark:hover:bg-neutral-800'
                }`}
                title="MCP / Skill"
                aria-label="MCP / Skill"
              >
                <SlidersHorizontal size={18} strokeWidth={1.75} />
              </button>
            )}

            <textarea
              ref={textareaRef}
              value={input}
              onChange={handleInput}
              onKeyDown={handleKeyDown}
              disabled={disabled}
              placeholder="Ask me anything..."
              rows={1}
              className="mb-0.5 max-h-40 min-h-[28px] flex-1 resize-none border-0 bg-transparent px-1 py-1.5 text-[15px] leading-relaxed text-neutral-900 outline-none placeholder:text-neutral-400 disabled:opacity-50 dark:text-neutral-100"
            />

            {cancelVisible && onCancel ? (
              <button
                type="button"
                onClick={onCancel}
                disabled={cancelling}
                className="chat-motion-fade-up mb-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-neutral-900 text-white shadow-sm transition-all hover:bg-neutral-700 disabled:bg-neutral-300 disabled:text-neutral-500 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200 dark:disabled:bg-neutral-700 dark:disabled:text-neutral-500"
                title={cancelling ? '正在停止' : '停止生成'}
                aria-label={cancelling ? '正在停止' : '停止生成'}
              >
                <Square size={13} strokeWidth={2.4} fill="currentColor" />
              </button>
            ) : (
              <button
                type="button"
                onClick={handleSend}
                disabled={!canSend}
                tabIndex={-1}
                title={sendDisabledReason || (canSend ? '发送' : '输入消息后发送')}
                aria-label={sendDisabledReason || '发送'}
                className={`mb-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-full transition-all ${
                  canSend
                    ? 'chat-motion-soft-pulse bg-[#e8a090] text-white shadow-sm hover:bg-[#df9585]'
                    : 'bg-neutral-200 text-neutral-400 dark:bg-neutral-700 dark:text-neutral-500'
                }`}
              >
                <ArrowUp size={18} strokeWidth={2.25} />
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}
