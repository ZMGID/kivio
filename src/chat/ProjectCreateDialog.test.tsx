import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import * as nativeDialog from '@tauri-apps/plugin-dialog'
import { ProjectCreateDialog } from './ProjectCreateDialog'

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))

beforeAll(() => {
  HTMLDialogElement.prototype.showModal ??= function () { this.setAttribute('open', '') }
})

beforeEach(() => {
  window.localStorage.clear()
  vi.mocked(nativeDialog.open).mockReset()
})

describe('ProjectCreateDialog', () => {
  it('turns a pasted GitHub shorthand into a clone with a derived name and remembered location', async () => {
    const user = userEvent.setup()
    const onCreate = vi.fn()
    vi.mocked(nativeDialog.open).mockResolvedValue('/Users/me/code')
    render(<ProjectCreateDialog onCreate={onCreate} onClose={vi.fn()} />)

    const search = screen.getByRole('combobox')
    expect(search).toHaveFocus()
    await user.type(search, 'owner/kivio-app{Enter}')

    expect(screen.getByLabelText('仓库地址')).toHaveValue('owner/kivio-app')
    expect(screen.getByLabelText('项目名称')).toHaveValue('kivio-app')
    const submit = screen.getByRole('button', { name: '克隆并创建' })
    expect(submit).toBeDisabled()

    await user.click(screen.getByRole('button', { name: '选择…' }))
    expect(await screen.findByText('/Users/me/code/kivio-app')).toBeInTheDocument()
    await user.click(submit)

    expect(onCreate).toHaveBeenCalledWith({
      kind: 'clone', name: 'kivio-app', url: 'owner/kivio-app', parentDir: '/Users/me/code',
    })
    expect(window.localStorage.getItem('kivio-project-parent-dir')).toBe('/Users/me/code')
  })

  it('keeps a name the user typed when the repository address changes', async () => {
    const user = userEvent.setup()
    render(<ProjectCreateDialog onCreate={vi.fn()} onClose={vi.fn()} />)
    await user.click(screen.getByRole('option', { name: /克隆 Git 仓库/ }))

    const url = screen.getByLabelText('仓库地址')
    expect(url).toHaveFocus()
    await user.type(url, 'https://github.com/a/first.git')
    expect(screen.getByLabelText('项目名称')).toHaveValue('first')
    await user.clear(screen.getByLabelText('项目名称'))
    await user.type(screen.getByLabelText('项目名称'), '我的项目')
    await user.clear(url)
    await user.type(url, 'git@github.com:a/second.git')
    expect(screen.getByLabelText('项目名称')).toHaveValue('我的项目')
  })

  it('navigates sources with the keyboard and creates a blank folder under the last location', async () => {
    const user = userEvent.setup()
    const onCreate = vi.fn()
    window.localStorage.setItem('kivio-project-parent-dir', '/work')
    render(<ProjectCreateDialog onCreate={onCreate} onClose={vi.fn()} />)

    await user.keyboard('{ArrowDown}')
    expect(screen.getByRole('option', { name: /新建空白文件夹/ })).toHaveAttribute('aria-selected', 'true')
    await user.keyboard('{Enter}')

    const name = screen.getByLabelText('项目名称')
    expect(name).toHaveFocus()
    await user.type(name, '../existing{Enter}')
    expect(onCreate).not.toHaveBeenCalled()
    await user.clear(name)
    await user.type(name, '发布计划{Enter}')
    expect(onCreate).toHaveBeenCalledWith({
      kind: 'local', name: '发布计划', rootPath: '/work/发布计划', ensureRootDir: true,
    })
  })

  it('filters sources, offers a folder path as a quick action, and returns to the list', async () => {
    const user = userEvent.setup()
    const onCreate = vi.fn()
    render(<ProjectCreateDialog onCreate={onCreate} onClose={vi.fn()} />)
    const search = screen.getByRole('combobox')

    await user.type(search, '对话')
    expect(screen.getAllByRole('option')).toHaveLength(1)
    await user.clear(search)
    await user.type(search, 'zzz')
    expect(screen.getByText('没有匹配的方式')).toBeInTheDocument()

    await user.clear(search)
    await user.type(search, '/tmp/site{Enter}')
    expect(screen.getByLabelText('项目名称')).toHaveValue('site')
    await user.click(screen.getByRole('button', { name: '返回' }))
    expect(screen.getByRole('combobox')).toHaveFocus()

    await user.click(screen.getByRole('option', { name: /仅整理对话/ }))
    await user.type(screen.getByLabelText('项目名称'), '杂项{Enter}')
    expect(onCreate).toHaveBeenCalledWith({ kind: 'local', name: '杂项', rootPath: null })
  })

  it('shows a save error only on the step that submitted it', async () => {
    const user = userEvent.setup()
    const { rerender } = render(<ProjectCreateDialog onCreate={vi.fn()} onClose={vi.fn()} />)
    await user.click(screen.getByRole('option', { name: /仅整理对话/ }))
    await user.type(screen.getByLabelText('项目名称'), '重复{Enter}')
    rerender(<ProjectCreateDialog error="项目名称已存在" onCreate={vi.fn()} onClose={vi.fn()} />)
    expect(screen.getByText('项目名称已存在')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: '返回' }))
    await user.click(screen.getByRole('option', { name: /新建空白文件夹/ }))
    expect(screen.queryByText('项目名称已存在')).not.toBeInTheDocument()
  })
})
