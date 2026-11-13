import { FolderIcon as FolderOpen } from '../components/FileIcon'
import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { open } from '@tauri-apps/plugin-dialog'
import { X } from 'lucide-react'
import { Button, IconButton } from '../components/Button'
import { Input } from '../settings/public/controls'
import type { ChatProject } from './types'
import { useCloseAnimation } from './useCloseAnimation'

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
  const title = project ? '编辑项目' : '新建项目'
  const { closing, startClose, onAnimationEnd } = useCloseAnimation(onClose)

  useEffect(() => {
    inputRef.current?.focus()
    inputRef.current?.select()
  }, [])

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') startClose()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [startClose])

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
        title: '选择项目文件夹',
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
      className={`${closing ? 'chat-motion-fade-out' : 'chat-motion-fade'} fixed inset-0 z-[300] flex items-center justify-center bg-neutral-900/30 px-4 backdrop-blur-[1px]`}
      data-tauri-drag-region="false"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) startClose()
      }}
    >
      <form
        className={`${closing ? 'chat-motion-modal-out' : 'chat-motion-modal-in'} w-full max-w-[340px] rounded-[10px] border border-neutral-200 bg-neutral-50 p-4 shadow-xl`}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onAnimationEnd={onAnimationEnd}
        onSubmit={(e) => {
          e.preventDefault()
          submit()
        }}
      >
        <h3 className="text-[14px] font-semibold text-neutral-900">{title}</h3>
        <label className="mt-3 block text-[12px] font-medium text-neutral-500">
          项目名称
        </label>
        <Input
          ref={inputRef}
          value={name}
          maxLength={80}
          onChange={setName}
          className="mt-1.5"
          placeholder="例如：产品发布计划"
        />
        <label className="mt-3 block text-[12px] font-medium text-neutral-500">
          项目文件夹
        </label>
        <div className="mt-1.5 flex min-w-0 items-center gap-1.5">
          {/* 与下拉触发器同款的字段外观：点击后由系统选择器取值。 */}
          <button
            type="button"
            onClick={pickFolder}
            disabled={saving}
            className="kv-select flex flex-1 items-center gap-2 text-left"
          >
            <FolderOpen open size={14} className="shrink-0 text-neutral-500" />
            <span className={`min-w-0 flex-1 truncate ${rootPath ? '' : 'text-neutral-400'}`}>
              {rootPath || '选择文件夹'}
            </span>
          </button>
          {rootPath && (
            <IconButton label="清除项目文件夹" onClick={() => setRootPath('')} disabled={saving}>
              <X />
            </IconButton>
          )}
        </div>
        {error && <p className="mt-2 text-[12px] text-danger">{error}</p>}
        <div className="mt-4 flex justify-end gap-2">
          <Button onClick={startClose}>取消</Button>
          <Button type="submit" variant="primary" disabled={!name.trim() || saving}>
            {saving ? '保存中…' : project ? '保存' : '创建'}
          </Button>
        </div>
      </form>
    </div>,
    document.body,
  )
}
