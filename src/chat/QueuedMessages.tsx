import { Clock3, X } from 'lucide-react'
import { IconButton } from '../components/Button'
import { isQueuedSubmitted, type QueuedMessage } from './hooks/useMessageQueue'
import type { Lang } from '../components/i18n'
import { i18n } from '../components/i18n'

interface QueuedMessagesProps {
  messages: QueuedMessage[]
  onRemove: (messageId: string) => void
  /** 点文本本体 = 撤回到输入框继续改。 */
  onRestore: (messageId: string) => void
  lang: Lang
}

/** Only unsent fallbacks remain beside the composer; submitted input lives in the transcript. */
export function QueuedMessages({
  messages,
  onRemove,
  onRestore,
  lang,
}: QueuedMessagesProps) {
  const waitingMessages = messages.filter(message => !isQueuedSubmitted(message))
  if (waitingMessages.length === 0) return null
  const t = i18n[lang]

  return (
    <div className="not-prose chat-composer-queue w-full">
      {/* 排太多条不许把输入框顶上去：约 4 行后内部滚动（custom-scrollbar = 全站同一根细条）。 */}
      <ul className="custom-scrollbar max-h-[7.5rem] overflow-y-auto p-1">
        {waitingMessages.map((message) => {
          // 撤回只把**文字**还给输入框（走 composer 的文本信道），所以带附件的那条不给撤 ——
          // 否则附件会静默消失。未提交时仍能删除。
          const restorable = message.attachments.length === 0
          return (
            <li
              key={message.id}
              className="chat-composer-queue-row flex h-7 items-center gap-1.5 rounded-md pl-1.5 pr-0.5"
            >
              <Clock3 size={12} strokeWidth={1.9} className="shrink-0 text-neutral-400 dark:text-neutral-500" />
              <button
                type="button"
                onClick={() => restorable && onRestore(message.id)}
                title={restorable ? t.chatQueueRestore : t.chatQueueRestoreBlocked}
                className={`min-w-0 flex-1 truncate text-left text-[12px] text-neutral-700 ${restorable ? '' : 'cursor-default'}`}
              >
                {message.content || t.chatQueuedAttachmentOnly}
                {message.attachments.length > 0 && (
                  <span className="ml-1.5 text-[11px] text-neutral-400 dark:text-neutral-500">
                    +{message.attachments.length}
                  </span>
                )}
              </button>
              {/* 引导被拒：这条得留在原地说话（不能只做 tooltip），否则用户点了按钮什么都看不见。 */}
              {message.steerRejected && (
                <span className="shrink-0 pr-1 text-[11px] text-amber-600 dark:text-amber-300">
                  {t.chatSteerRejected}
                </span>
              )}
              {message.followUpRejected && (
                <span className="shrink-0 pr-1 text-[11px] text-amber-600 dark:text-amber-300">
                  {t.chatFollowUpRejected}
                </span>
              )}
              <span className="chat-composer-queue-actions flex shrink-0 items-center gap-0.5">
                  <IconButton
                    size="xs"
                    variant="ghost"
                    label={t.chatQueueRemove}
                    onClick={() => onRemove(message.id)}
                  >
                    <X size={12} />
                  </IconButton>
              </span>
            </li>
          )
        })}
      </ul>
    </div>
  )
}
