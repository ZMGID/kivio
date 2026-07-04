import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { open } from '@tauri-apps/plugin-dialog'
import { FolderOpen, X } from 'lucide-react'
import type { ChatProject } from './types'

interface ProjectDialogProps {
  project?: ChatProject | null
  saving?: boolean
  error?: string
  onSave: (name: string, rootPath?: string | null) => void
  onClose: () => void
}

export function ProjectDialog({
  project,
  saving = false,
  error = '',
  onSave,
  onClose,
}: ProjectDialogProps) {
  const [name, setName] = useState(project?.name ?? '')
  const [rootPath, setRootPath] = useState(project?.root_path ?? project?.rootPath ?? '')
  const inputRef = useRef<HTMLInputElement>(null)
  const title = project ? '編輯專案' : '新建專案'

  useEffect(() => {
    inputRef.current?.focus()
    inputRef.current?.select()
  }, [])

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [onClose])

  const submit = () => {
    const nextName = name.trim()
    if (!nextName || saving) return
    onSave(nextName, rootPath.trim() || null)
  }

  const pickFolder = async () => {
    if (saving) return
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: '選擇專案資料夾',
      })
      const path = Array.isArray(picked) ? picked[0] : picked
      if (!path) return
      setRootPath(path)
      if (!name.trim()) {
        const fallbackName = path.split(/[\\/]/).filter(Boolean).pop()
        if (fallbackName) setName(fallbackName)
      }
    } catch (err) {
      console.error('Failed to pick project folder:', err)
    }
  }

  return createPortal(
    <div
      className="chat-motion-fade fixed inset-0 z-[300] flex items-center justify-center bg-black/30 px-4 backdrop-blur-[1px]"
      data-tauri-drag-region="false"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <form
        className="chat-motion-modal-in w-full max-w-[340px] rounded-[10px] border border-neutral-200 bg-white p-4 shadow-xl dark:border-neutral-700 dark:bg-[#252527]"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onSubmit={(e) => {
          e.preventDefault()
          submit()
        }}
      >
        <h3 className="text-[14px] font-semibold text-neutral-900 dark:text-neutral-50">{title}</h3>
        <label className="mt-3 block text-[12px] font-medium text-neutral-500 dark:text-neutral-400">
          專案名稱
        </label>
        <input
          ref={inputRef}
          type="text"
          value={name}
          maxLength={80}
          onChange={(e) => setName(e.target.value)}
          className="mt-1.5 w-full rounded-lg border border-neutral-200 bg-white px-3 py-2 text-[13px] text-neutral-900 outline-none transition-colors placeholder:text-neutral-400 focus:border-neutral-400 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
          placeholder="例如：產品釋出計畫"
        />
        <label className="mt-3 block text-[12px] font-medium text-neutral-500 dark:text-neutral-400">
          專案資料夾
        </label>
        <div className="mt-1.5 flex min-w-0 gap-2">
          <button
            type="button"
            onClick={pickFolder}
            disabled={saving}
            className="flex min-w-0 flex-1 items-center gap-2 rounded-lg border border-neutral-200 bg-white px-3 py-2 text-left text-[13px] text-neutral-700 outline-none transition-colors hover:border-neutral-300 disabled:cursor-default disabled:opacity-50 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-200 dark:hover:border-neutral-600"
          >
            <FolderOpen size={15} strokeWidth={1.75} className="shrink-0 text-neutral-500" />
            <span className={`min-w-0 flex-1 truncate ${rootPath ? '' : 'text-neutral-400'}`}>
              {rootPath || '選擇資料夾'}
            </span>
          </button>
          {rootPath && (
            <button
              type="button"
              onClick={() => setRootPath('')}
              disabled={saving}
              className="shrink-0 rounded-lg border border-neutral-200 bg-white p-2 text-neutral-400 transition-colors hover:border-neutral-300 hover:text-neutral-700 disabled:cursor-default disabled:opacity-50 dark:border-neutral-700 dark:bg-neutral-900 dark:hover:border-neutral-600 dark:hover:text-neutral-200"
              aria-label="清除專案資料夾"
              title="清除專案資料夾"
            >
              <X size={15} strokeWidth={1.75} />
            </button>
          )}
        </div>
        {error && <p className="mt-2 text-[12px] text-red-600 dark:text-red-400">{error}</p>}
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg px-3 py-1.5 text-[13px] text-neutral-600 transition-colors hover:bg-black/[0.04] dark:text-neutral-300 dark:hover:bg-white/[0.06]"
          >
            取消
          </button>
          <button
            type="submit"
            disabled={!name.trim() || saving}
            className="rounded-lg bg-neutral-900 px-3 py-1.5 text-[13px] font-medium text-white transition-colors hover:bg-neutral-800 disabled:cursor-default disabled:opacity-50 dark:bg-neutral-100 dark:text-neutral-950 dark:hover:bg-white"
          >
            {saving ? '儲存中…' : project ? '儲存' : '建立'}
          </button>
        </div>
      </form>
    </div>,
    document.body,
  )
}
