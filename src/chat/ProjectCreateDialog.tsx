import { useEffect, useId, useRef, useState, type ReactNode } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { ArrowLeft, CornerDownLeft, FolderOpen, FolderPlus, Github, MessagesSquare, Search, X, type LucideIcon } from 'lucide-react'
import { Button, IconButton } from '../components/Button'
import { FolderIcon } from '../components/FileIcon'
import { useT } from '../components/i18n'
import { FieldBlock, Input, KeyBadge } from '../settings/public/controls'
import { useCloseAnimation } from './useCloseAnimation'

/** 新建项目的请求：本地文件夹 / 空白文件夹 / 无文件夹走 createProject，克隆走 cloneProject。 */
export type ProjectCreateRequest =
  | { kind: 'local'; name: string; rootPath: string | null; ensureRootDir?: boolean }
  | { kind: 'clone'; name: string; url: string; parentDir: string }

type Step = 'sources' | 'folder' | 'blank' | 'clone' | 'plain'
type Glyph = LucideIcon
interface PaletteItem {
  id: string
  icon: Glyph
  title: string
  detail: string
  keywords?: string
  run: () => void
}

/** 空白文件夹与克隆共用的上次存放位置，只用于预填。 */
const PARENT_DIR_KEY = 'kivio-project-parent-dir'

function folderName(path: string) {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path
}

function joinPath(parent: string, name: string) {
  const sep = parent.includes('\\') && !parent.includes('/') ? '\\' : '/'
  return `${parent.replace(/[\\/]+$/, '')}${sep}${name}`
}

// 以下判断只决定是否给出「快速操作」建议；地址与路径是否有效由后端判定。
function looksLikeGitAddress(value: string) {
  return /^(https?:\/\/|ssh:\/\/|git:\/\/|git@|github\.com\/)\S+$/i.test(value)
    || /^[\w-][\w.-]*\/[\w-][\w.-]*$/.test(value)
}

function looksLikeLocalPath(value: string) {
  return /^(\/|~[\\/]|[a-zA-Z]:[\\/])/.test(value)
}

function repoNameFromUrl(url: string) {
  const tail = url.trim().replace(/[\\/]+$/, '').split(/[/:]/).pop() ?? ''
  return tail.replace(/\.git$/i, '')
}

function readParentDir() {
  try {
    return window.localStorage.getItem(PARENT_DIR_KEY) ?? ''
  } catch {
    return ''
  }
}

function rememberParentDir(path: string) {
  try {
    window.localStorage.setItem(PARENT_DIR_KEY, path)
  } catch {
    // 存不下只是下次不预填。
  }
}

async function pickDirectory(title: string) {
  try {
    const picked = await open({ directory: true, multiple: false, title })
    return (Array.isArray(picked) ? picked[0] : picked) || null
  } catch (err) {
    console.error('Failed to pick folder:', err)
    return null
  }
}

interface ProjectCreateDialogProps {
  saving?: boolean
  error?: string
  onCreate: (request: ProjectCreateRequest) => void
  onClose: () => void
}

/**
 * 新建项目：先选来源（本地文件夹 / 空白文件夹 / 克隆仓库 / 仅整理对话），再补名称与位置。
 * 搜索框可直接粘贴 Git 地址或文件夹路径，作为快速操作置顶。
 */
export function ProjectCreateDialog({ saving = false, error = '', onCreate, onClose }: ProjectCreateDialogProps) {
  const t = useT()
  const [step, setStep] = useState<Step>('sources')
  const [query, setQuery] = useState('')
  const [active, setActive] = useState(0)
  const [name, setName] = useState('')
  const [nameTouched, setNameTouched] = useState(false)
  const [rootPath, setRootPath] = useState('')
  const [parentDir, setParentDir] = useState(readParentDir)
  const [url, setUrl] = useState('')
  const [picking, setPicking] = useState(false)
  // 错误只在发起提交的那一步显示，返回换一种方式后不再残留。
  const [submittedStep, setSubmittedStep] = useState<Step | null>(null)
  const dialogRef = useRef<HTMLDialogElement>(null)
  const bodyRef = useRef<HTMLDivElement>(null)
  const searchRef = useRef<HTMLInputElement>(null)
  const pressedBackdrop = useRef(false)
  const titleId = useId()
  const listId = useId()
  const formId = useId()
  const { closing, startClose, onAnimationEnd } = useCloseAnimation(onClose)

  useEffect(() => {
    const dialog = dialogRef.current
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null
    if (dialog && !dialog.open) dialog.showModal?.()
    return () => {
      if (previous?.isConnected) previous.focus()
    }
  }, [])

  // 换步后把焦点放到该步的第一个输入框。
  useEffect(() => {
    if (step === 'sources') {
      searchRef.current?.focus()
      return
    }
    bodyRef.current?.querySelector<HTMLInputElement>('input[data-autofocus]')?.focus()
  }, [step])

  const goTo = (next: Step) => {
    setSubmittedStep(null)
    setStep(next)
  }

  const back = () => {
    if (saving) return
    goTo('sources')
  }

  const withPicker = async (title: string) => {
    if (picking) return null
    setPicking(true)
    try {
      return await pickDirectory(title)
    } finally {
      setPicking(false)
    }
  }

  const openLocalFolder = async () => {
    const path = await withPicker(t.chatPickProjectFolder)
    if (!path) return
    adoptFolder(path)
  }

  const adoptFolder = (path: string) => {
    setRootPath(path)
    setName(folderName(path))
    goTo('folder')
  }

  const changeFolder = async () => {
    const path = await withPicker(t.chatPickProjectFolder)
    if (!path) return
    // 名称还是旧文件夹名时跟着换，用户改过就保留。
    if (!name.trim() || name === folderName(rootPath)) setName(folderName(path))
    setRootPath(path)
  }

  const chooseLocation = async () => {
    const path = await withPicker(t.projectCreatePickLocation)
    if (path) setParentDir(path)
  }

  const startFresh = (next: Step) => {
    setName('')
    setNameTouched(false)
    goTo(next)
  }

  const startClone = (address = '') => {
    setUrl(address)
    setName(repoNameFromUrl(address))
    setNameTouched(false)
    goTo('clone')
  }

  const trimmedQuery = query.trim()
  const quickItems: PaletteItem[] = []
  if (looksLikeGitAddress(trimmedQuery)) {
    quickItems.push({
      id: 'quick-clone',
      icon: Github,
      title: t.projectCreateCloneThis.replace('{value}', () => trimmedQuery),
      detail: t.projectCreateCloneThisDetail,
      run: () => startClone(trimmedQuery),
    })
  } else if (looksLikeLocalPath(trimmedQuery)) {
    quickItems.push({
      id: 'quick-folder',
      icon: FolderOpen,
      title: t.projectCreateUseFolderThis.replace('{value}', () => trimmedQuery),
      detail: t.projectCreateUseFolderThisDetail,
      run: () => adoptFolder(trimmedQuery),
    })
  }
  const sources: PaletteItem[] = [
    {
      id: 'local',
      icon: FolderOpen,
      title: t.projectCreateLocalTitle,
      detail: t.projectCreateLocalDetail,
      keywords: 'local folder open directory 本地 文件夹 打开 目录',
      run: () => void openLocalFolder(),
    },
    {
      id: 'blank',
      icon: FolderPlus,
      title: t.projectCreateBlankTitle,
      detail: t.projectCreateBlankDetail,
      keywords: 'new blank empty create 新建 空白 创建',
      run: () => startFresh('blank'),
    },
    {
      id: 'clone',
      icon: Github,
      title: t.projectCreateCloneTitle,
      detail: t.projectCreateCloneDetail,
      keywords: 'git github clone url repository repo 克隆 仓库',
      run: () => startClone(),
    },
    {
      id: 'plain',
      icon: MessagesSquare,
      title: t.projectCreatePlainTitle,
      detail: t.projectCreatePlainDetail,
      keywords: 'plain chat conversation none 对话 无文件夹 整理',
      run: () => startFresh('plain'),
    },
  ]
  const needle = trimmedQuery.toLowerCase()
  const matchedSources = quickItems.length > 0
    ? sources
    : sources.filter((item) => !needle
      || `${item.title} ${item.detail} ${item.keywords ?? ''}`.toLowerCase().includes(needle))
  const items = [...quickItems, ...matchedSources]
  const activeIndex = Math.min(active, Math.max(items.length - 1, 0))
  const optionId = (index: number) => `${listId}-${index}`

  const invalidFolderName = (step === 'blank' || step === 'clone')
    && (/[\\/:*?"<>|\p{Cc}]/u.test(name) || name.trimEnd().endsWith('.'))
  const nameValid = Boolean(name.trim()) && !invalidFolderName
  const canSubmit = !saving && !picking && nameValid && (
    step === 'folder' ? Boolean(rootPath)
      : step === 'blank' ? Boolean(parentDir)
        : step === 'clone' ? Boolean(url.trim() && parentDir)
          : step === 'plain'
  )

  const submit = () => {
    if (!canSubmit) return
    const trimmedName = name.trim()
    setSubmittedStep(step)
    if (step === 'folder') {
      onCreate({ kind: 'local', name: trimmedName, rootPath })
    } else if (step === 'blank') {
      rememberParentDir(parentDir)
      onCreate({ kind: 'local', name: trimmedName, rootPath: joinPath(parentDir, trimmedName), ensureRootDir: true })
    } else if (step === 'clone') {
      rememberParentDir(parentDir)
      onCreate({ kind: 'clone', name: trimmedName, url: url.trim(), parentDir })
    } else if (step === 'plain') {
      onCreate({ kind: 'local', name: trimmedName, rootPath: null })
    }
  }

  const stepMeta: Record<Exclude<Step, 'sources'>, { icon: Glyph; title: string }> = {
    folder: { icon: FolderOpen, title: t.projectCreateLocalTitle },
    blank: { icon: FolderPlus, title: t.projectCreateBlankTitle },
    clone: { icon: Github, title: t.projectCreateCloneTitle },
    plain: { icon: MessagesSquare, title: t.projectCreatePlainTitle },
  }

  const nameField = (
    <FieldBlock label={t.projectCreateName} htmlFor={`${formId}-name`}>
      <Input
        id={`${formId}-name`}
        value={name}
        maxLength={80}
        aria-invalid={invalidFolderName || undefined}
        aria-describedby={invalidFolderName ? `${formId}-name-error` : undefined}
        onChange={(value) => {
          setName(value)
          setNameTouched(true)
        }}
        placeholder={t.projectCreateNamePlaceholder}
        data-autofocus={step === 'clone' ? undefined : true}
        disabled={saving}
      />
      {invalidFolderName && <p id={`${formId}-name-error`} className="kv-inline-error">{t.projectCreateInvalidFolderName}</p>}
    </FieldBlock>
  )

  const pathCard = (path: string, empty: string, action: ReactNode) => (
    <div className="flex min-w-0 items-center gap-2.5 rounded-[10px] border border-[var(--border)] bg-[var(--bg-input-subtle)] py-2 pl-2.5 pr-2">
      <div className="grid size-8 shrink-0 place-items-center rounded-lg bg-[var(--bg)] shadow-[var(--shadow-sm)]" aria-hidden="true">
        <FolderIcon open={Boolean(path)} size={15} />
      </div>
      <div className="min-w-0 flex-1">
        {path ? (
          <>
            <div className="truncate text-[13px] font-medium leading-[18px]">{folderName(path)}</div>
            <div className="truncate text-[11px] leading-4 text-[var(--text-muted)]" title={path}>{path}</div>
          </>
        ) : (
          <div className="text-[13px] text-[var(--text-muted)]">{empty}</div>
        )}
      </div>
      {action}
    </div>
  )

  const locationField = (
    <FieldBlock label={t.projectCreateLocation}>
      {pathCard(parentDir, t.projectCreateNoLocation, (
        <Button size="sm" onClick={() => void chooseLocation()} disabled={saving || picking}>
          {parentDir ? t.projectCreateChange : t.projectCreateChooseLocation}
        </Button>
      ))}
      {parentDir && name.trim() && (
        <p className="mt-1.5 truncate text-[11.5px] text-[var(--text-muted)]">
          {t.projectCreateWillCreate}
          <span className="ml-1 font-mono text-[var(--text)]">{joinPath(parentDir, name.trim())}</span>
        </p>
      )}
    </FieldBlock>
  )

  const hint = (keys: string[], label: string) => (
    <span className="inline-flex items-center gap-1">
      {keys.map((key) => <KeyBadge key={key}>{key}</KeyBadge>)}
      <span>{label}</span>
    </span>
  )

  const StepIcon = step === 'sources' ? null : stepMeta[step].icon

  return (
    <dialog
      ref={dialogRef}
      className={`kv kv-modal overflow-hidden${closing ? ' is-closing' : ''}`}
      aria-labelledby={titleId}
      style={{ width: 'min(520px, calc(100vw - 32px))', maxWidth: 520, maxHeight: 'calc(100dvh - 48px)', padding: 0 }}
      data-tauri-drag-region="false"
      onAnimationEnd={onAnimationEnd}
      onCancel={(e) => {
        e.preventDefault()
        if (step !== 'sources' && !saving) back()
        else startClose()
      }}
      onMouseDown={(e) => {
        pressedBackdrop.current = e.target === e.currentTarget
      }}
      onClick={(e) => {
        const fromBackdrop = pressedBackdrop.current && e.target === e.currentTarget
        pressedBackdrop.current = false
        if (fromBackdrop) startClose()
      }}
    >
      <div className="flex max-h-[calc(100dvh-48px)] min-h-0 flex-col">
        <header className="flex h-[52px] shrink-0 items-center gap-2 border-b border-[var(--border)] pl-3 pr-2.5">
          {step === 'sources' ? (
            <>
              <h2 id={titleId} className="sr-only">{t.chatNewProject}</h2>
              <Search size={16} strokeWidth={1.75} className="ml-1 shrink-0 text-[var(--text-muted)]" aria-hidden="true" />
              <input
                ref={searchRef}
                value={query}
                onChange={(e) => {
                  setQuery(e.target.value)
                  setActive(0)
                }}
                onKeyDown={(e) => {
                  if (e.nativeEvent.isComposing) return
                  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                    e.preventDefault()
                    if (items.length === 0) return
                    const delta = e.key === 'ArrowDown' ? 1 : -1
                    setActive((activeIndex + delta + items.length) % items.length)
                  } else if (e.key === 'Enter') {
                    e.preventDefault()
                    items[activeIndex]?.run()
                  }
                }}
                role="combobox"
                aria-expanded="true"
                aria-controls={listId}
                aria-activedescendant={items.length > 0 ? optionId(activeIndex) : undefined}
                aria-label={t.projectCreateSearch}
                placeholder={t.projectCreateSearch}
                className="min-w-0 flex-1 border-0 bg-transparent text-[14px] text-[var(--text)] outline-none placeholder:text-neutral-400"
                spellCheck={false}
                autoComplete="off"
              />
            </>
          ) : (
            <>
              <IconButton label={t.projectCreateBack} variant="ghost" onClick={back} disabled={saving}>
                <ArrowLeft size={16} strokeWidth={1.75} />
              </IconButton>
              {StepIcon && <StepIcon size={16} strokeWidth={1.75} className="shrink-0 text-[var(--accent)]" />}
              <h2 id={titleId} className="min-w-0 flex-1 truncate text-[14px] font-semibold">
                {stepMeta[step].title}
              </h2>
            </>
          )}
          <IconButton label={t.projectCreateKeyClose} variant="ghost" onClick={startClose}>
            <X size={16} strokeWidth={1.75} />
          </IconButton>
        </header>

        <div ref={bodyRef} key={step} className="chat-motion-fade custom-scrollbar min-h-0 flex-1 overflow-y-auto">
          {step === 'sources' ? (
            <div id={listId} role="listbox" aria-label={t.chatNewProject} className="p-2">
              {items.length === 0 && (
                <p className="px-3 py-6 text-center text-[12.5px] text-[var(--text-muted)]">{t.projectCreateNoMatch}</p>
              )}
              {items.map((item, index) => {
                const Icon = item.icon
                const selected = index === activeIndex
                const sectionLabel = index === 0 && quickItems.length > 0
                  ? t.projectCreateQuickSection
                  : index === quickItems.length ? t.projectCreateSourcesSection : null
                return (
                  <div key={item.id}>
                    {sectionLabel && (
                      <div className={`px-3 pb-1 text-[11px] font-medium text-[var(--text-muted)] ${index === 0 ? 'pt-1' : 'pt-3'}`}>
                        {sectionLabel}
                      </div>
                    )}
                    <button
                      id={optionId(index)}
                      type="button"
                      role="option"
                      aria-selected={selected}
                      tabIndex={-1}
                      disabled={picking}
                      onMouseMove={() => {
                        if (!selected) setActive(index)
                      }}
                      onClick={item.run}
                      className={`flex w-full min-w-0 items-center gap-3 rounded-lg px-2.5 py-2 text-left transition-colors ${
                        selected ? 'bg-[var(--bg-hover)]' : ''
                      }`}
                    >
                      <span
                        className={`grid size-8 shrink-0 place-items-center rounded-lg border border-[var(--border)] bg-[var(--bg)] transition-colors ${
                          selected ? 'text-[var(--accent)]' : 'text-[var(--text-muted)]'
                        }`}
                        aria-hidden="true"
                      >
                        <Icon size={16} strokeWidth={1.75} />
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-[13px] font-medium leading-[18px]">{item.title}</span>
                        <span className="block truncate text-[12px] leading-[17px] text-[var(--text-muted)]">{item.detail}</span>
                      </span>
                      {selected && (
                        <CornerDownLeft size={14} strokeWidth={1.75} className="shrink-0 text-[var(--text-muted)]" aria-hidden="true" />
                      )}
                    </button>
                  </div>
                )
              })}
            </div>
          ) : (
            <form
              id={formId}
              className="px-5 pb-3 pt-2"
              onSubmit={(e) => {
                e.preventDefault()
                submit()
              }}
            >
              {step === 'clone' && (
                <FieldBlock label={t.projectCreateRepoUrl} htmlFor={`${formId}-url`}>
                  <Input
                    id={`${formId}-url`}
                    value={url}
                    mono
                    onChange={(value) => {
                      setUrl(value)
                      if (!nameTouched) setName(repoNameFromUrl(value))
                    }}
                    placeholder={t.projectCreateRepoPlaceholder}
                    data-autofocus
                    disabled={saving}
                    spellCheck={false}
                    autoComplete="off"
                  />
                </FieldBlock>
              )}
              {nameField}
              {step === 'folder' && (
                <FieldBlock label={t.projectCreateFolder}>
                  {pathCard(rootPath, '', (
                    <Button size="sm" variant="ghost" onClick={() => void changeFolder()} disabled={saving || picking}>
                      {t.projectCreateChange}
                    </Button>
                  ))}
                </FieldBlock>
              )}
              {(step === 'blank' || step === 'clone') && locationField}
              {step === 'plain' && (
                <p className="pb-1 text-[12px] text-[var(--text-muted)]">{t.projectCreatePlainHint}</p>
              )}
              {error && submittedStep === step && <p className="kv-inline-error">{error}</p>}
            </form>
          )}
        </div>

        <footer className="flex min-h-[44px] shrink-0 items-center gap-3 border-t border-[var(--border)] bg-[var(--bg-savebar)] px-4 py-2 text-[11.5px] text-[var(--text-muted)]">
          {step === 'sources' ? (
            <>
              {hint(['↑', '↓'], t.projectCreateKeyNavigate)}
              {hint(['Enter'], t.projectCreateKeyConfirm)}
              {hint(['Esc'], t.projectCreateKeyClose)}
            </>
          ) : (
            <>
              {hint(['Enter'], t.projectCreateSubmit)}
              {hint(['Esc'], t.projectCreateKeyBack)}
              <span className="flex-1" />
              <Button type="submit" form={formId} variant="primary" disabled={!canSubmit}>
                {saving
                  ? step === 'clone' ? t.projectCreateCloning : t.projectCreateCreating
                  : step === 'clone' ? t.projectCreateCloneSubmit : t.projectCreateSubmit}
              </Button>
            </>
          )}
        </footer>
      </div>
    </dialog>
  )
}
