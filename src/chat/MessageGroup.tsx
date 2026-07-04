import { memo, useMemo, useState, type ReactNode } from 'react'
import { Check, Columns2, Square } from 'lucide-react'
import type { ChatMessage } from './types'
import { MessageBubble } from './MessageBubble'
import { ModelIcon } from './ModelIcon'
import { getActiveGroup, useGroupsVersion, type GroupColumnSnapshot } from './groupStreamingStore'
import { useMultiAnswerViewMode } from './multiAnswerViewMode'

// 多模型一問多答（任務 06-30 / 步驟 6 + 8）：把同一 group_id 的 N 條 assistant 答案展示出來。
// 兩種來源互斥：
//  - 流式中（sendMessage 未返回）：列來自 groupStreamingStore 的即時列（live=true）。
//  - 落庫後：列來自持久化的 assistant 訊息（live=false），各帶 group_id / provider_id / model。
// virtua 把整組當「一行」item 虛擬化（見 MessageList），不破壞滾動/釘底。
//
// 兩種展示模式（全域偏好 useMultiAnswerViewMode，預設 'tabs'）：
//  - 'tabs'（切換）：一次只整寬顯示**當前選中條**（預設第一條），組末尾 footer 切換顯示哪條。
//  - 'columns'（並排）：N 列橫向並排（原有實現，視覺/效能完全不變）。
// 組末尾 footer：檢視切換控制元件 + 一排模型 chip（點 chip = 切顯示條 +「續聊選中條」一舉兩用）。
//
// 效能降級（步驟 8 / R10）：N 列同時全量渲染 reasoning + markdown 是記憶體/CPU 大頭。
// 「聚焦列」（hover 的列 / tabs 模式當前顯示列）展開 reasoning 流式；其餘「非聚焦列」把
// reasoningStreaming 置 false → ReasoningBlock 摺疊並把正文從 DOM 解除安裝（hideBody）。
// 複用既有 KaTeX Shadow DOM / rAF 合幀（touchGroup）/ virtua 屏外解除安裝，不重複造輪子。

interface MessageGroupProps {
  conversationId?: string | null
  groupId: string
  // 落庫後的本組 assistant 訊息（順序即列序）；流式中為空。
  messages: ChatMessage[]
  // 當前組的選中條 message id（D5）；空時預設第一列。
  selectedMessageId?: string | null
  onSelectColumn?: (groupId: string, messageId: string) => void
  onUpdateMessage?: (messageId: string, content: string) => Promise<void>
  onRegenerateMessage?: (messageId: string) => Promise<void>
  onDeleteMessage?: (messageId: string) => Promise<void>
}

interface GroupColumn {
  message: ChatMessage
  streaming: boolean
}

function columnModelLabel(provider: string | null | undefined, model: string | null | undefined): string {
  const m = (model ?? '').trim()
  const p = (provider ?? '').trim()
  if (m && p) return `${m} | ${p}`
  return m || p || '模型'
}

function streamColumnToMessage(column: GroupColumnSnapshot): ChatMessage {
  return {
    id: column.messageId,
    role: 'assistant',
    content: column.content,
    reasoning: column.reasoning || undefined,
    artifacts: [],
    tool_calls: column.toolCalls,
    segments: column.segments,
    provider_id: column.providerId,
    model: column.model,
    timestamp: Math.floor(Date.now() / 1000),
  }
}

// 列內滾動框：用原生滾動 + CSS `overscroll-contain`（滾到列邊界不串聯到外層列表）。
// 不要用 JS wheel 監聽手動改 scrollTop——那會繞過瀏覽器合成器的平滑/慣性滾動，導致掉幀。
function ColumnScrollBody({ children }: { children: ReactNode }) {
  return (
    <div className="chat-message-group-col-body custom-scrollbar min-h-0 flex-1 overflow-y-auto overscroll-contain">
      {children}
    </div>
  )
}

// 單列容器：並排模式（columns）和切換模式（tabs）共用同一個 MessageBubble 渲染，
// 僅外層佈局/邊框/選中態不同（通過 props 控制）。
function GroupColumnView({
  column,
  conversationId,
  live,
  isSelected,
  isFocused,
  showColumnChrome,
  groupId,
  onMouseEnter,
  onSelectColumn,
  onUpdateMessage,
  onRegenerateMessage,
  onDeleteMessage,
}: {
  column: GroupColumn
  conversationId?: string | null
  live: boolean
  isSelected: boolean
  isFocused: boolean
  // columns 模式渲染列頭（model 標籤 + 「用這條繼續」按鈕）；tabs 模式列頭交給 footer chip。
  showColumnChrome: boolean
  groupId: string
  onMouseEnter?: () => void
  onSelectColumn?: (groupId: string, messageId: string) => void
  onUpdateMessage?: (messageId: string, content: string) => Promise<void>
  onRegenerateMessage?: (messageId: string) => Promise<void>
  onDeleteMessage?: (messageId: string) => Promise<void>
}) {
  const { message, streaming } = column
  const wrapperClass = showColumnChrome
    ? `chat-message-group-col flex max-h-[min(560px,70vh)] min-w-[280px] max-w-[420px] flex-1 flex-col rounded-2xl border px-3 py-2 ${
        isSelected
          ? 'border-emerald-400/70 bg-emerald-50/40 dark:border-emerald-500/50 dark:bg-emerald-950/20'
          : 'border-neutral-200/70 bg-neutral-50/40 dark:border-neutral-700/60 dark:bg-neutral-900/30'
      }`
    : 'chat-message-group-tab flex w-full flex-col'
  return (
    <div
      onMouseEnter={onMouseEnter}
      className={wrapperClass}
      data-chat-message-group-focused={isFocused ? 'true' : undefined}
    >
      {showColumnChrome && (
        <div className="mb-1 flex items-center justify-between gap-2">
          <div className="flex min-w-0 items-center gap-1.5 text-[11px] font-medium text-neutral-500 dark:text-neutral-400">
            {message.model && <ModelIcon model={message.model} size={14} />}
            <span className="min-w-0 truncate" title={columnModelLabel(message.provider_id, message.model)}>
              {columnModelLabel(message.provider_id, message.model)}
            </span>
          </div>
          {!live && onSelectColumn && (
            <button
              type="button"
              onClick={() => onSelectColumn(groupId, message.id)}
              aria-pressed={isSelected}
              title={isSelected ? '已選為續聊上下文' : '用這條繼續'}
              className={`inline-flex shrink-0 items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-medium transition-colors ${
                isSelected
                  ? 'bg-emerald-500/90 text-white'
                  : 'border border-neutral-200 text-neutral-500 hover:bg-neutral-100 dark:border-neutral-700 dark:text-neutral-400 dark:hover:bg-neutral-800'
              }`}
            >
              <Check size={11} strokeWidth={2.5} />
              {isSelected ? '已選' : '用這條繼續'}
            </button>
          )}
        </div>
      )}
      {/* 列內滾動框 + 滾動隔離：頭部固定，正文超列高時列內豎向滾動；遊標在哪列就滾哪列
          （ColumnScrollBody 非 passive wheel 監聽），到邊界再交給外層列表。
          tabs 模式整寬顯示，不限列高，交給外層列表滾動（不套 ColumnScrollBody）。 */}
      {showColumnChrome ? (
        <ColumnScrollBody>
          <MessageBubble
            message={message}
            conversationId={conversationId}
            messageStreaming={streaming}
            // 效能降級（R10）：非聚焦列把 reasoningStreaming 置 false，讓 ReasoningBlock 摺疊
            // 並把思維鏈正文從 DOM 解除安裝（hideBody）；聚焦列正常展示流式思考。
            reasoningStreaming={streaming && isFocused}
            onUpdateMessage={!live ? onUpdateMessage : undefined}
            onRegenerateMessage={!live ? onRegenerateMessage : undefined}
            onDeleteMessage={!live ? onDeleteMessage : undefined}
          />
        </ColumnScrollBody>
      ) : (
        <MessageBubble
          message={message}
          conversationId={conversationId}
          messageStreaming={streaming}
          // tabs 模式：當前顯示列即聚焦列 → 正常展示流式思考。
          reasoningStreaming={streaming && isFocused}
          onUpdateMessage={!live ? onUpdateMessage : undefined}
          onRegenerateMessage={!live ? onRegenerateMessage : undefined}
          onDeleteMessage={!live ? onDeleteMessage : undefined}
        />
      )}
    </div>
  )
}

// 組末尾切換欄（參考 Cherry）：檢視切換控制元件 + 一排模型 chip。
function GroupFooter({
  columns,
  viewMode,
  onChangeViewMode,
  activeMessageId,
  markContext,
  onSelectChip,
}: {
  columns: GroupColumn[]
  viewMode: 'tabs' | 'columns'
  onChangeViewMode: (mode: 'tabs' | 'columns') => void
  // 當前高亮的那條（tabs：正顯示的；columns：續聊選中條）。
  activeMessageId: string | null
  // 高亮 chip 是否代表「已選為下一輪上下文」（落庫後為 true；流式中上下文未定為 false）。
  markContext: boolean
  onSelectChip: (messageId: string) => void
}) {
  return (
    <div className="chat-message-group-footer mt-2 flex flex-wrap items-center gap-2 border-t border-neutral-200/60 pt-2.5 dark:border-neutral-700/50">
      {/* 檢視切換：iOS 風分段控制元件，啟用項白底浮起，剋制不搶眼 */}
      <div className="inline-flex shrink-0 items-center rounded-lg bg-neutral-100 p-0.5 dark:bg-neutral-800/60">
        {([
          ['tabs', Square, '切換', '切換顯示（一次一條）'],
          ['columns', Columns2, '並排', '並排顯示（多列）'],
        ] as const).map(([mode, Icon, label, hint]) => (
          <button
            key={mode}
            type="button"
            onClick={() => onChangeViewMode(mode)}
            aria-pressed={viewMode === mode}
            title={hint}
            className={`inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium transition-colors ${
              viewMode === mode
                ? 'bg-white text-neutral-800 shadow-sm dark:bg-neutral-700 dark:text-neutral-100'
                : 'text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300'
            }`}
          >
            <Icon size={12} strokeWidth={2} />
            {label}
          </button>
        ))}
      </div>
      {/* 模型 chip：圖示 + 模型名（短，無邊框），當前顯示/選中的淡綠高亮 */}
      <div className="flex min-w-0 flex-wrap items-center gap-0.5">
        {columns.map(({ message }) => {
          const isActive = message.id === activeMessageId
          const shortLabel = (message.model ?? '').trim() || columnModelLabel(message.provider_id, message.model)
          return (
            <button
              key={message.id}
              type="button"
              onClick={() => onSelectChip(message.id)}
              aria-pressed={isActive}
              title={columnModelLabel(message.provider_id, message.model)}
              className={`inline-flex max-w-[160px] shrink-0 items-center gap-1.5 rounded-full px-2.5 py-1 text-[11px] font-medium transition-colors ${
                isActive
                  ? 'bg-emerald-500/10 text-emerald-700 dark:text-emerald-300'
                  : 'text-neutral-500 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800'
              }`}
            >
              {message.model && <ModelIcon model={message.model} size={13} />}
              <span className="min-w-0 truncate">{shortLabel}</span>
              {isActive && markContext && (
                <Check size={12} strokeWidth={2.5} className="shrink-0 text-emerald-600 dark:text-emerald-400" />
              )}
            </button>
          )
        })}
      </div>
    </div>
  )
}

function MessageGroupBase({
  conversationId,
  groupId,
  messages,
  selectedMessageId,
  onSelectColumn,
  onUpdateMessage,
  onRegenerateMessage,
  onDeleteMessage,
}: MessageGroupProps) {
  // 訂閱 group store 版本號：流式列內容更新時驅動重渲。
  useGroupsVersion()
  const liveGroup = conversationId ? getActiveGroup(conversationId) : undefined
  const live = Boolean(liveGroup && liveGroup.groupId === groupId)

  // 全域展示模式偏好（預設 tabs）。
  const [viewMode, setViewMode] = useMultiAnswerViewMode()

  // 聚焦列索引（效能降級 R10，僅 columns 模式用）：hover 哪一列就聚焦哪一列；預設聚焦第一列。
  const [focusedIndex, setFocusedIndex] = useState(0)
  // tabs 模式當前顯示的列 message id（未顯式選 → 跟隨選中條 / 第一條）。
  const [tabMessageId, setTabMessageId] = useState<string | null>(null)

  const columns = useMemo<GroupColumn[]>(() => {
    if (live && liveGroup) {
      return liveGroup.columns.map((col) => ({
        message: streamColumnToMessage(col),
        streaming: col.streaming,
      }))
    }
    return messages.map((message) => ({ message, streaming: false }))
  }, [live, liveGroup, messages])

  if (columns.length === 0) return null

  // 選中列：有顯式記錄用它；否則預設第一列（D5）。流式態不顯示選中標記（還沒落庫）。
  const effectiveSelectedId = selectedMessageId || (columns[0]?.message.id ?? null)

  // tabs 模式當前顯示哪條：使用者在本組點過 chip → tabMessageId（若仍存在於列裡）；
  // 否則跟隨續聊選中條 / 第一條。流式列認領後 message id 會從佔位變真實，故做存在性校驗。
  const tabActiveId =
    (tabMessageId && columns.some((c) => c.message.id === tabMessageId) ? tabMessageId : null) ??
    effectiveSelectedId
  const tabColumn = columns.find((c) => c.message.id === tabActiveId) ?? columns[0]

  // footer chip 點選：tabs 模式切顯示條；並落到續聊選中條（onSelectColumn，落庫態才有意義）。
  const handleChipClick = (messageId: string) => {
    setTabMessageId(messageId)
    if (!live && onSelectColumn) onSelectColumn(groupId, messageId)
  }

  // footer 高亮的那條：tabs 看正顯示的；columns 看續聊選中條（流式態無選中 → 不高亮）。
  const footerActiveId = viewMode === 'tabs' ? tabColumn.message.id : (live ? null : effectiveSelectedId)

  return (
    <div className="chat-message-group-wrap flex w-full flex-col py-2">
      {viewMode === 'columns' ? (
        <div className="chat-message-group custom-scrollbar flex w-full gap-3 overflow-x-auto pb-1">
          {columns.map((column, index) => (
            <GroupColumnView
              key={column.message.id}
              column={column}
              conversationId={conversationId}
              live={live}
              isSelected={!live && column.message.id === effectiveSelectedId}
              isFocused={index === focusedIndex}
              showColumnChrome
              groupId={groupId}
              onMouseEnter={() => setFocusedIndex(index)}
              onSelectColumn={onSelectColumn}
              onUpdateMessage={onUpdateMessage}
              onRegenerateMessage={onRegenerateMessage}
              onDeleteMessage={onDeleteMessage}
            />
          ))}
        </div>
      ) : (
        <GroupColumnView
          key={tabColumn.message.id}
          column={tabColumn}
          conversationId={conversationId}
          live={live}
          isSelected={false}
          // tabs 當前顯示列即聚焦列（正常展示流式思考）。
          isFocused
          showColumnChrome={false}
          groupId={groupId}
          onSelectColumn={onSelectColumn}
          onUpdateMessage={onUpdateMessage}
          onRegenerateMessage={onRegenerateMessage}
          onDeleteMessage={onDeleteMessage}
        />
      )}
      <GroupFooter
        columns={columns}
        viewMode={viewMode}
        onChangeViewMode={setViewMode}
        activeMessageId={footerActiveId}
        markContext={!live}
        onSelectChip={handleChipClick}
      />
    </div>
  )
}

export const MessageGroup = memo(MessageGroupBase)
