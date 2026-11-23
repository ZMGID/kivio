import { FolderIcon } from '../components/FileIcon'
import { useEffect, useId, useRef, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { FolderPlus, X } from 'lucide-react'
import { Button, IconButton } from '../components/Button'
import { FieldBlock, Input } from '../settings/public/controls'
import type { ChatProject } from './types'
import { useCloseAnimation } from './useCloseAnimation'

interface ProjectDialogProps {
  project: ChatProject
  saving?: boolean
  error?: string
  onSave: (name: string, rootPath?: string | null) => void
  onClose: () => void
}

function folderName(path: string) {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path
}

/** 编辑项目：名称必填；项目文件夹可选，选后若名称为空则用文件夹名补上。新建走 ProjectCreateDialog。 */
export function ProjectDialog({
  project,
  saving = false,
  error = '',
  onSave,
  onClose,
}: ProjectDialogProps) {
  const [name, setName] = useState(project.name)
  const [rootPath, setRootPath] = useState(project.root_path ?? project.rootPath ?? '')
  const dialogRef = useRef<HTMLDialogElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  // 在对话框内按下、拖到遮罩上松开（例如选中文字）不算点遮罩。
  const pressedBackdrop = useRef(false)
  const titleId = useId()
  const nameId = useId()
  const title = '编辑项目'
  const { closing, startClose, onAnimationEnd } = useCloseAnimation(onClose)

  useEffect(() => {
    const dialog = dialogRef.current
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null
    if (dialog && !dialog.open) dialog.showModal?.()
    inputRef.current?.focus()
    inputRef.current?.select()
    return () => {
      if (previous?.isConnected) previous.focus()
    }
  }, [])

  const canSubmit = Boolean(name.trim()) && !saving
  const submit = () => {
    if (!canSubmit) return
    onSave(name.trim(), rootPath.trim() || null)
  }

  const pickFolder = async () => {
    if (saving) return
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: '选择项目文件夹',
      })
      const path = Array.isArray(picked) ? picked[0] : picked
      if (!path) return
      setRootPath(path)
      if (!name.trim()) setName(folderName(path))
    } catch (err) {
      console.error('Failed to pick project folder:', err)
    }
  }

  return (
    <dialog
      ref={dialogRef}
      className={`kv kv-modal overflow-hidden${closing ? ' is-closing' : ''}`}
      aria-labelledby={titleId}
      style={{ width: 'min(440px, calc(100vw - 32px))', maxWidth: 440, maxHeight: 'calc(100dvh - 48px)', padding: 0 }}
      data-tauri-drag-region="false"
      onAnimationEnd={onAnimationEnd}
      onCancel={(e) => {
        e.preventDefault()
        startClose()
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
      <form
        className="flex max-h-[calc(100dvh-48px)] min-h-0 flex-col"
        onSubmit={(e) => {
          e.preventDefault()
          submit()
        }}
      >
        <header className="flex shrink-0 items-start gap-3 px-5 pb-3 pt-5">
          <div
            className="grid size-10 shrink-0 place-items-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent)]"
            aria-hidden="true"
          >
            <FolderIcon open={Boolean(rootPath)} size={19} className="!text-[var(--accent)]" />
          </div>
          <div className="min-w-0 flex-1 pt-0.5">
            <h2 id={titleId} className="truncate text-[15px] font-semibold leading-5">
              {title}
            </h2>
            <p className="mt-0.5 text-[12px] leading-[18px] text-[var(--text-muted)]">
              把相关对话和工作文件夹放在一起
            </p>
          </div>
          <IconButton label="关闭" variant="ghost" onClick={startClose}>
            <X size={16} strokeWidth={1.75} />
          </IconButton>
        </header>

        <div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto px-5 pb-3">
          <FieldBlock label="项目名称" htmlFor={nameId}>
            <Input
              ref={inputRef}
              id={nameId}
              value={name}
              maxLength={80}
              onChange={setName}
              placeholder="例如：产品发布计划"
            />
          </FieldBlock>

          <FieldBlock
            label={<>
              项目文件夹
              <span className="ml-1.5 font-normal text-neutral-400">可选</span>
            </>}
          >
            {rootPath ? (
              <div className="flex min-w-0 items-center gap-2.5 rounded-[10px] border border-[var(--border)] bg-[var(--bg-input-subtle)] py-2 pl-2.5 pr-1.5">
                <div className="grid size-8 shrink-0 place-items-center rounded-lg bg-[var(--bg)] shadow-[var(--shadow-sm)]" aria-hidden="true">
                  <FolderIcon open size={15} />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[13px] font-medium leading-[18px]">{folderName(rootPath)}</div>
                  <div className="truncate text-[11px] leading-4 text-[var(--text-muted)]" title={rootPath}>
                    {rootPath}
                  </div>
                </div>
                <Button size="sm" variant="ghost" onClick={pickFolder} disabled={saving}>
                  更换
                </Button>
                <IconButton label="清除项目文件夹" variant="ghost" onClick={() => setRootPath('')} disabled={saving}>
                  <X size={14} strokeWidth={1.75} />
                </IconButton>
              </div>
            ) : (
              <button
                type="button"
                onClick={pickFolder}
                disabled={saving}
                aria-label="选择文件夹"
                className="group flex w-full flex-col items-center gap-1.5 rounded-[10px] border border-dashed border-[var(--border-strong)] bg-[var(--bg-input-subtle)] px-4 py-5 text-center transition-colors hover:border-[var(--accent)] hover:bg-[color-mix(in_srgb,var(--accent)_5%,var(--bg))] disabled:opacity-50"
              >
                <span className="grid size-9 place-items-center rounded-full bg-[var(--bg)] text-[var(--text-muted)] shadow-[var(--shadow-sm)] transition-colors group-hover:text-[var(--accent)]">
                  <FolderPlus size={17} strokeWidth={1.75} />
                </span>
                <span className="text-[13px] font-medium">选择文件夹</span>
                <span className="text-[11.5px] text-[var(--text-muted)]">项目内的对话将在此文件夹中工作</span>
              </button>
            )}
          </FieldBlock>

          {error && <p className="kv-inline-error">{error}</p>}
        </div>

        <footer className="flex shrink-0 items-center justify-end gap-2 border-t border-[var(--border)] bg-[var(--bg-savebar)] px-5 py-3">
          <Button onClick={startClose}>取消</Button>
          <Button type="submit" variant="primary" disabled={!canSubmit}>
            {saving ? '保存中…' : '保存'}
          </Button>
        </footer>
      </form>
    </dialog>
  )
}
