import { FolderIcon as Folder, FolderIcon as FolderOpen } from '../components/FileIcon'
import { ErrorNotice } from '../components/AppNotice'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useWindowStore } from '../utils/windowStore'
import { ArrowLeft, Check, ChevronLeft, FolderInput, FolderPlus, NotebookPen, Pencil, Plus, Trash2 } from 'lucide-react'
import { Crepe } from '@milkdown/crepe'
import '@milkdown/crepe/theme/common/style.css'
import '@milkdown/crepe/theme/frame.css'
import { api, isTauriRuntime, type NoteMeta } from '../api/tauri'
import { Button, IconButton } from '../components/Button'
import { Input } from '../settings/public/controls'
import { workspaceActivity } from './dock/workspaceActivity'
import { useT } from '../components/i18n'
import { confirmDialog } from '../components/dialogQueue'
import {
  discardNoteDraft,
  editNoteDraft,
  flushNoteDraft,
  noteDraftStore,
  settleNoteDelete,
  showNote,
} from './notesDraftStore'

let noteNavigation = 0

function claimNoteNavigation() {
  noteNavigation += 1
  return noteNavigation
}

function noteNavigationCurrent(request: number) {
  return request === noteNavigation
}

/**
 * Obsidian 风格的一体化写作面：Milkdown Crepe 提供 markdown 原生 live-preview
 * （输入 `# ` 直接成标题、**粗体** 内联渲染，光标行才露语法）。Crepe 是非受控编辑器，
 * defaultValue 只设一次，靠 markdownUpdated 回传变更；切笔记时用 key 重挂即可。
 */
function MilkdownNoteEditor({
  initialMarkdown,
  onChange,
  flushMarkdownRef,
}: {
  initialMarkdown: string
  onChange: (markdown: string) => void
  flushMarkdownRef: { current: (() => void) | null }
}) {
  const rootRef = useRef<HTMLDivElement>(null)
  const onChangeRef = useRef(onChange)
  onChangeRef.current = onChange

  useEffect(() => {
    const el = rootRef.current
    if (!el) return
    const crepe = new Crepe({ root: el, defaultValue: initialMarkdown })
    let active = true
    let created = false
    let publishedMarkdown = initialMarkdown
    const flush = () => {
      if (!active || !created) return
      const markdown = crepe.getMarkdown()
      if (markdown === publishedMarkdown) return
      publishedMarkdown = markdown
      onChangeRef.current(markdown)
    }
    flushMarkdownRef.current = flush
    crepe.on((listener) => {
      listener.markdownUpdated((_ctx, markdown) => {
        if (!active) return
        publishedMarkdown = markdown
        onChangeRef.current(markdown)
      })
    })
    const ready = crepe.create().then(() => {
      if (!active) return
      created = true
      publishedMarkdown = crepe.getMarkdown()
    })
    return () => {
      active = false
      if (flushMarkdownRef.current === flush) flushMarkdownRef.current = null
      void ready.then(() => crepe.destroy())
    }
    // 挂载一次；切笔记由外层 key 触发重挂
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return <div ref={rootRef} className="kv-note-editor min-h-full" />
}

function displayTitle(title: string | undefined, untitled: string): string {
  return title?.trim() || untitled
}

export function NotesCenter() {
  const t = useT()
  const [notes, setNotes] = useState<NoteMeta[]>([])
  const [folders, setFolders] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  const [search, setSearch] = useState('')

  // 当前文件夹：null = 根目录（显示文件夹 + 散笔记），字符串 = 进入该文件夹
  const [currentFolder, setCurrentFolder] = useState<string | null>(null)
  // 文件夹命名弹框：WKWebView 不支持 window.prompt（恒返回 null），故用内联输入。
  // 输入走非受控 ref，规避中文 IME 合成期受控写回吞字（与编辑器同款处理）。
  const [folderDialog, setFolderDialog] = useState<
    | { mode: 'create'; assignNoteId?: string }
    | { mode: 'rename'; original: string }
    | null
  >(null)
  const folderInputRef = useRef<HTMLInputElement>(null)
  // 「移动到文件夹」菜单：当前展开的笔记 id。
  const [moveMenuFor, setMoveMenuFor] = useState<string | null>(null)

  // 打开的笔记、草稿和保存队列在窗口 store 里，离开页面后仍在。
  // 标题/正文输入保持非受控：受控 input 在中文 IME 合成期被 React 写回 value 会打断输入。
  const [draft] = useWindowStore(noteDraftStore)
  const editing = draft.note
  const flushMarkdownRef = useRef<(() => void) | null>(null)
  const saving = draft.status === 'saving'

  const loadNotes = useCallback(async () => {
    setError('')
    try {
      const [list, folderList] = await Promise.all([api.notesList(), api.notesFoldersList()])
      setNotes(list)
      setFolders(folderList)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setLoading(false)
    }
  }, [])

  /**
   * 打开笔记目录，让用户把外部 `.md` 拖进来。目录是扁平的、`parse_note` 对缺
   * frontmatter 的手工文件有回退（文件名当标题、mtime 当时间），所以不需要导入
   * 流程——目录监听（见下方 effect）会在文件落地后自动刷新列表。
   */
  const openNotesFolder = useCallback(async () => {
    setError('')
    try {
      await api.notesOpenFolder()
    } catch (err) {
      setError(err instanceof Error ? err.message : t.chatNotesOpenFolderFailed)
    }
  }, [t])

  useEffect(() => {
    if (!isTauriRuntime()) {
      setLoading(false)
      setError(t.chatNotesAppOnly)
      return
    }
    void loadNotes()
  }, [loadNotes, t])

  /**
   * 监听笔记目录本身：用户把外部 `.md` 拖进来（或在别处编辑、删除）后自动重读，
   * 不需要手动刷新。复用 dock 的 workspace watcher（notify 递归监听 + 250ms 去抖
   * + 2s 轮询兜底），而不是自己起一份监听——那套去抖/兜底是实测调过的。
   * 注意 `subscribe` 的后端语义是「整体替换 watch 集合」，而 workspaceActivity
   * 模块内部按订阅方合并全集，所以这里与 dock 的 workdir 订阅可以共存。
   */
  useEffect(() => {
    if (!isTauriRuntime()) return
    let cancelled = false
    let unsubscribe: (() => void) | null = null
    void api
      .notesDirPath()
      .then((dir) => {
        if (cancelled || !dir) return
        unsubscribe = workspaceActivity.subscribe(dir, (event) => {
          // 编辑期跳过：正在编辑的笔记是我们自己在防抖写盘，重读会把列表状态
          // 拽回去；退出编辑时 backToList 已经自己 loadNotes 了。
          if (noteDraftStore.getSnapshot().note) return
          if (event.fs || event.truncated) void loadNotes()
        })
      })
      .catch(() => {
        // 拿不到目录就退化为「打开文件夹按钮 + 重进页面刷新」，不打扰用户。
      })
    return () => {
      cancelled = true
      unsubscribe?.()
    }
  }, [loadNotes])

  const visibleNotes = useMemo(() => {
    const needle = search.trim().toLowerCase()
    return notes.filter((note) => needle
      ? displayTitle(note.title, t.chatNotesUntitled).toLowerCase().includes(needle) || note.preview.toLowerCase().includes(needle)
      : note.folder.trim() === (currentFolder ?? ''))
  }, [notes, currentFolder, search, t])

  // 离开笔记页不是取消：把未落盘的草稿交给窗口里的保存队列，回来仍打开同一篇。
  useEffect(() => () => {
    claimNoteNavigation()
    flushMarkdownRef.current?.()
    void flushNoteDraft()
  }, [])

  /** 编辑器回传：草稿进窗口 store，保存队列在页面之外继续。 */
  const onEditorChange = useCallback((markdown: string) => {
    if (noteDraftStore.getSnapshot().session !== draft.session) return
    editNoteDraft({ content: markdown })
  }, [draft.session])

  const openNote = useCallback(async (id: string) => {
    const request = claimNoteNavigation()
    const session = noteDraftStore.getSnapshot().session
    flushMarkdownRef.current?.()
    if (!await flushNoteDraft()) return
    if (!noteNavigationCurrent(request)) return
    if (noteDraftStore.getSnapshot().session !== session) return
    setError('')
    try {
      const note = await api.notesRead(id)
      if (!noteNavigationCurrent(request)) return
      showNote(note)
    } catch (err) {
      if (!noteNavigationCurrent(request)) return
      setError(err instanceof Error ? err.message : String(err))
    }
  }, [])

  const backToList = useCallback(async () => {
    const request = claimNoteNavigation()
    const session = noteDraftStore.getSnapshot().session
    flushMarkdownRef.current?.()
    if (!await flushNoteDraft()) return
    if (!noteNavigationCurrent(request)) return
    if (noteDraftStore.getSnapshot().session !== session) return
    discardNoteDraft()
    void loadNotes()
  }, [loadNotes])

  const createNote = useCallback(async () => {
    const request = claimNoteNavigation()
    const session = noteDraftStore.getSnapshot().session
    flushMarkdownRef.current?.()
    if (!await flushNoteDraft()) return
    if (!noteNavigationCurrent(request)) return
    if (noteDraftStore.getSnapshot().session !== session) return
    setError('')
    const folder = currentFolder ?? ''
    try {
      const note = await api.notesCreate('', '', folder, 'user')
      if (!noteNavigationCurrent(request)) return
      await loadNotes()
      if (!noteNavigationCurrent(request)) return
      showNote(note)
    } catch (err) {
      if (!noteNavigationCurrent(request)) return
      setError(err instanceof Error ? err.message : String(err))
    }
  }, [loadNotes, currentFolder])

  const deleteNote = useCallback(
    async (id: string) => {
      const meta = notes.find((n) => n.id === id)
      const ok = await confirmDialog({
        message: t.chatNotesDeleteNoteConfirm.replace('{title}', () => displayTitle(meta?.title, t.chatNotesUntitled)),
        confirmLabel: t.dialogDelete,
        danger: true,
      })
      if (!ok) return
      setError('')
      try {
        await settleNoteDelete(id, () => api.notesDelete(id))
        setNotes((prev) => prev.filter((n) => n.id !== id))
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      }
    },
    [notes, t],
  )

  /* ===== 文件夹管理 ===== */
  const moveNoteToFolder = useCallback(
    async (id: string, folder: string) => {
      setMoveMenuFor(null)
      setError('')
      try {
        const note = await api.notesRead(id)
        if (note.folder === folder) return
        await api.notesUpdate(id, note.title, note.content, folder)
        await loadNotes()
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      }
    },
    [loadNotes],
  )

  const createFolder = useCallback(() => {
    setFolderDialog({ mode: 'create' })
  }, [])

  const renameFolder = useCallback((name: string) => {
    setFolderDialog({ mode: 'rename', original: name })
  }, [])

  const submitFolderDialog = useCallback(async () => {
    if (!folderDialog) return
    const name = (folderInputRef.current?.value ?? '').trim()
    if (!name) return
    if (folderDialog.mode === 'rename' && name === folderDialog.original) {
      setFolderDialog(null)
      return
    }
    setError('')
    try {
      if (folderDialog.mode === 'create') {
        setFolders(await api.notesFolderCreate(name))
        // 若来自卡片「新建文件夹并移入」，创建后把该笔记归入新文件夹。
        if (folderDialog.assignNoteId) {
          const note = await api.notesRead(folderDialog.assignNoteId)
          await api.notesUpdate(note.id, note.title, note.content, name)
          await loadNotes()
        }
      } else {
        await api.notesFolderRename(folderDialog.original, name)
        if (currentFolder === folderDialog.original) setCurrentFolder(name)
        await loadNotes()
      }
      setFolderDialog(null)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }, [folderDialog, currentFolder, loadNotes])

  const deleteFolder = useCallback(
    async (name: string) => {
      const ok = await confirmDialog({
        message: t.chatNotesDeleteFolderConfirm.replace('{name}', () => name),
        confirmLabel: t.dialogDelete,
        danger: true,
      })
      if (!ok) return
      setError('')
      try {
        await api.notesFolderDelete(name)
        if (currentFolder === name) setCurrentFolder(null)
        await loadNotes()
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      }
    },
    [currentFolder, loadNotes, t],
  )

  /* ===== 编辑器态 ===== */
  if (editing) {
    return (
      <div className="assistant-center-root flex h-full min-h-0 flex-col text-neutral-900">
        <div className="mx-auto flex h-full w-full min-h-0 max-w-[820px] flex-col px-9 pb-4 pt-6">
          <div className="flex shrink-0 items-center justify-between gap-3">
            <Button variant="ghost" size="sm" onClick={() => void backToList()}>
              <ArrowLeft size={14} />
              {t.chatNotesBack}
            </Button>
            <div className="flex shrink-0 items-center gap-2">
              <span className="text-[12px] text-neutral-400 dark:text-neutral-500">
                {saving ? t.annotateSaving : t.saved}
              </span>
              <IconButton
                size="sm"
                variant="ghost"
                label={t.chatDelete}
                onClick={() => void deleteNote(editing.id)}
              >
                <Trash2 size={15} />
              </IconButton>
            </div>
          </div>

          <input
            key={`${editing.id}:${draft.session}`}
            type="text"
            defaultValue={draft.title}
            onChange={(e) => {
              editNoteDraft({ title: e.target.value })
            }}
            placeholder={t.chatNotesUntitled}
            className="mt-5 w-full shrink-0 bg-transparent text-[26px] font-semibold tracking-normal text-neutral-950 placeholder:text-neutral-300 focus:outline-none dark:placeholder:text-neutral-600"
          />
          <div className="custom-scrollbar mt-3 min-h-0 flex-1 overflow-y-auto">
            <MilkdownNoteEditor
              key={`${editing.id}:${draft.session}`}
              initialMarkdown={draft.content}
              onChange={onEditorChange}
              flushMarkdownRef={flushMarkdownRef}
            />
          </div>

          {(draft.error || error) && (
            <div className="mt-3 shrink-0 rounded-md border border-red-200 bg-red-50 px-4 py-2.5 text-[13px] text-red-700 dark:border-red-900/50 dark:bg-red-950/30 dark:text-red-300">
              {draft.error || error}
            </div>
          )}
        </div>
      </div>
    )
  }

  /* ===== 列表态 ===== */
  const atRoot = currentFolder === null
  const showFolders = atRoot && !search.trim() && folders.length > 0
  const emptyEverything = !showFolders && visibleNotes.length === 0

  return (
    <div className="assistant-center-root flex h-full min-h-0 flex-col text-neutral-900">
      <main className="custom-scrollbar min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto w-full max-w-[1040px] px-9 pb-10 pt-7">
          {/* 笔记 */}
          <div className="border-b border-neutral-200 pb-5">
            <h1 className="flex items-center gap-2.5 text-[28px] font-semibold tracking-normal text-neutral-950">
              <NotebookPen size={24} className="text-neutral-500" />
              {t.chatNavNotes}
            </h1>
          </div>

          {/* 搜索与新建 */}
          <div className="mt-5 flex flex-wrap items-center gap-3">
            <div className="w-full max-w-xs">
              <Input
                value={search}
                onChange={setSearch}
                placeholder={t.chatNotesSearchPlaceholder}
                aria-label={t.chatNotesSearchPlaceholder}
              />
            </div>
            <div className="ml-auto flex shrink-0 items-center gap-2">
              <Button variant="ghost" onClick={() => void openNotesFolder()}>
                <FolderOpen open size={14} />
                {t.chatNotesOpenFolder}
              </Button>
              {atRoot && (
                <Button variant="ghost" onClick={() => void createFolder()}>
                  <FolderPlus size={14} />
                  {t.dockNewFolder}
                </Button>
              )}
              <Button onClick={() => void createNote()}>
                <Plus size={14} />
                {t.chatNotesNewNote}
              </Button>
            </div>
          </div>

          {/* 返回根目录 */}
          {currentFolder !== null && (
            <button
              type="button"
              onClick={() => setCurrentFolder(null)}
              className="mt-4 inline-flex items-center gap-1 text-[13px] text-neutral-500 hover:text-neutral-800 dark:text-neutral-400"
            >
              <ChevronLeft size={15} />
              {t.chatNavNotes}
              <span className="text-neutral-300 dark:text-neutral-600">/</span>
              <span className="font-medium text-neutral-700">{currentFolder}</span>
            </button>
          )}

          {error && (
            <ErrorNotice message={error} />
          )}

          {loading && notes.length === 0 ? (
            <div className="mt-6 flex flex-col gap-3">
              {Array.from({ length: 3 }, (_, i) => (
                <div key={i} className="kv-skeleton h-7 w-full rounded" />
              ))}
            </div>
          ) : (
            <>
              {/* 文件夹 */}
              {showFolders && (
                <div className="mt-5 flex flex-col gap-1">
                  {folders.map((name) => (
                    <div
                      key={name}
                      role="button"
                      tabIndex={0}
                      onClick={() => setCurrentFolder(name)}
                      onKeyDown={(e) => {
                        if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) {
                          e.preventDefault()
                          setCurrentFolder(name)
                        }
                      }}
                      className="group flex cursor-pointer items-center gap-3 rounded-md px-3 py-2 hover:bg-neutral-100"
                    >
                      <Folder size={20} className="shrink-0 text-neutral-400 dark:text-neutral-500" />
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-[14px] font-medium text-neutral-900">
                          {name}
                        </div>
                      </div>
                      <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
                        <IconButton
                          size="xs"
                          variant="ghost"
                          label={t.chatRename}
                          onClick={(e) => {
                            e.stopPropagation()
                            void renameFolder(name)
                          }}
                        >
                          <Pencil size={13} />
                        </IconButton>
                        <IconButton
                          size="xs"
                          variant="ghost"
                          label={t.chatDelete}
                          onClick={(e) => {
                            e.stopPropagation()
                            void deleteFolder(name)
                          }}
                        >
                          <Trash2 size={13} />
                        </IconButton>
                      </div>
                    </div>
                  ))}
                </div>
              )}

              {/* 空状态 */}
              {emptyEverything ? (
                <div className="mt-16 flex flex-col items-center justify-center text-center">
                  <div className="flex h-14 w-14 items-center justify-center rounded-md bg-neutral-100 text-neutral-400 dark:text-neutral-500">
                    <NotebookPen size={28} strokeWidth={1.5} />
                  </div>
                  <p className="mt-4 text-[15px] font-medium text-neutral-700">
                    {search.trim() ? t.chatNotesNoMatch : currentFolder ? t.chatNotesEmptyFolder : t.chatNotesEmptyRecent}
                  </p>
                </div>
              ) : (
                visibleNotes.length > 0 && (
                  <div className="mt-3 flex flex-col gap-1">
                    {visibleNotes.map((note) => (
                      <article
                        key={note.id}
                        role="button"
                        tabIndex={0}
                        onClick={() => void openNote(note.id)}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter' || e.key === ' ') {
                            e.preventDefault()
                            void openNote(note.id)
                          }
                        }}
                        className="group min-w-0 cursor-pointer rounded-md px-3 py-2 hover:bg-neutral-100"
                      >
                        <div className="flex min-w-0 items-center justify-between gap-2">
                          <NotebookPen size={16} className="shrink-0 text-neutral-400" />
                          <h3 className="min-w-0 flex-1 truncate text-[14px] font-medium text-neutral-900">
                            {displayTitle(note.title, t.chatNotesUntitled)}
                          </h3>
                          <div className="flex shrink-0 items-center gap-0.5">
                            <div className="relative">
                              <IconButton
                                size="xs"
                                variant="ghost"
                                label={t.chatNotesMoveToFolder}
                                className={`transition-opacity ${moveMenuFor === note.id ? 'opacity-100' : 'opacity-0 group-hover:opacity-100 group-focus-within:opacity-100'}`}
                                onClick={(e) => {
                                  e.stopPropagation()
                                  setMoveMenuFor((prev) => (prev === note.id ? null : note.id))
                                }}
                              >
                                <FolderInput size={13} />
                              </IconButton>
                              {moveMenuFor === note.id && (
                                <>
                                  <div
                                    className="fixed inset-0 z-40"
                                    onClick={(e) => {
                                      e.stopPropagation()
                                      setMoveMenuFor(null)
                                    }}
                                  />
                                  <div
                                    className="custom-scrollbar absolute right-0 z-50 mt-1 max-h-64 w-44 overflow-auto kv-menu"
                                    onClick={(e) => e.stopPropagation()}
                                  >
                                    <button
                                      type="button"
                                      className="kv-menu-item"
                                      onClick={() => void moveNoteToFolder(note.id, '')}
                                    >
                                      {note.folder.trim() === '' && <Check size={12} className="text-accent" />}
                                      <span className={note.folder.trim() === '' ? '' : 'ml-[18px]'}>{t.chatNotesLibraryRoot}</span>
                                    </button>
                                    {folders.map((f) => (
                                      <button
                                        key={f}
                                        type="button"
                                        className="kv-menu-item truncate"
                                        onClick={() => void moveNoteToFolder(note.id, f)}
                                      >
                                        {note.folder.trim() === f && <Check size={12} className="text-accent" />}
                                        <span className={`truncate ${note.folder.trim() === f ? '' : 'ml-[18px]'}`}>{f}</span>
                                      </button>
                                    ))}
                                    <div className="my-1 border-t border-neutral-100" />
                                    <button
                                      type="button"
                                      className="kv-menu-item"
                                      onClick={() => {
                                        setMoveMenuFor(null)
                                        setFolderDialog({ mode: 'create', assignNoteId: note.id })
                                      }}
                                    >
                                      <FolderPlus size={12} />
                                      {t.chatNotesNewFolderEllipsis}
                                    </button>
                                  </div>
                                </>
                              )}
                            </div>
                            <IconButton
                              size="xs"
                              variant="ghost"
                              label={t.chatDelete}
                              className={`transition-opacity ${moveMenuFor === note.id ? 'opacity-100' : 'opacity-0 group-hover:opacity-100 group-focus-within:opacity-100'}`}
                              onClick={(e) => {
                                e.stopPropagation()
                                void deleteNote(note.id)
                              }}
                            >
                              <Trash2 size={13} />
                            </IconButton>
                          </div>
                        </div>
                      </article>
                    ))}
                  </div>
                )
              )}
            </>
          )}
        </div>
      </main>

      {folderDialog && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-neutral-900/30 px-4"
          onMouseDown={() => setFolderDialog(null)}
        >
          <div
            className="w-full max-w-xs rounded-xl border border-neutral-200 bg-neutral-50 p-4 shadow-xl"
            onMouseDown={(e) => e.stopPropagation()}
          >
            <h3 className="text-[14px] font-semibold text-neutral-900">
              {folderDialog.mode === 'create' ? t.dockNewFolder : t.chatRename}
            </h3>
            <input
              ref={folderInputRef}
              autoFocus
              defaultValue={folderDialog.mode === 'rename' ? folderDialog.original : ''}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault()
                  void submitFolderDialog()
                }
                if (e.key === 'Escape') setFolderDialog(null)
              }}
              placeholder={t.chatNotesFolderNamePlaceholder}
              className="mt-3 w-full rounded-lg border border-neutral-300 bg-neutral-50 px-3 py-2 text-[13px] text-neutral-900 outline-none focus:border-accent dark:border-neutral-600"
            />
            <div className="mt-4 flex justify-end gap-2">
              <Button variant="ghost" size="sm" onClick={() => setFolderDialog(null)}>
                {t.cancel}
              </Button>
              <Button size="sm" onClick={() => void submitFolderDialog()}>
                {t.chatNotesConfirm}
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
