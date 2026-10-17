import { getComposerDraft, setComposerDraft } from './composerDraft'

type Listener = (text: string) => void

// Cached chat and center pages can coexist; the latest mounted composer owns insertion.
const listeners: Listener[] = []

export function onComposerInsert(cb: Listener): () => void {
  listeners.push(cb)
  return () => {
    const index = listeners.indexOf(cb)
    if (index >= 0) listeners.splice(index, 1)
  }
}

export function insertIntoComposer(text: string): void {
  listeners[listeners.length - 1]?.(text)
}

// 文本直插信道（Right Dock「插入 @ 引用」等）：与上面的引用卡片信道并列，
// 区别是文本直接进输入框正文而不是挂成引用卡片。卸载后恢复之前挂载的输入框。
const textListeners: { listener: Listener; draftKey?: string }[] = []

export function onComposerTextInsert(cb: Listener, draftKey?: string): () => void {
  const registration = { listener: cb, draftKey }
  textListeners.push(registration)
  return () => {
    const index = textListeners.indexOf(registration)
    if (index >= 0) textListeners.splice(index, 1)
  }
}

export function insertTextIntoComposer(text: string): void {
  if (!text) return
  const active = textListeners[textListeners.length - 1]
  const textDraftKey = active?.draftKey
  // 文本信号和会话切换可能发生在同一次 React 更新里（首条「回到这里」会从底栏
  // InputBar 切到欢迎页 InputBar）。只 setState 会把文字留在即将卸载的旧组件里；先同步写入
  // 当前草稿，使新挂载的 InputBar 也能读到。随后 listener 仍负责即时更新未卸载的输入框。
  if (textDraftKey) {
    const current = getComposerDraft(textDraftKey) ?? { input: '', quotes: [], attachments: [] }
    const needsSpace = current.input.length > 0
      && !current.input.endsWith(' ')
      && !current.input.endsWith('\n')
    setComposerDraft(textDraftKey, {
      ...current,
      input: `${current.input}${needsSpace ? ' ' : ''}${text}`,
    })
  }
  active?.listener(text)
}
