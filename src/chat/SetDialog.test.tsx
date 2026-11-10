import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import { SetDialog } from './SetDialog'
import type { ChatAssistant } from './types'

beforeAll(() => {
  HTMLDialogElement.prototype.showModal ??= function () { this.setAttribute('open', '') }
})

const assistants = [{ id: 'writer', name: '写作助手' }] as ChatAssistant[]

describe('SetDialog', () => {
  it('creates a set with only a name; the other fields stay optional', async () => {
    const user = userEvent.setup()
    const onSave = vi.fn()
    render(<SetDialog assistants={assistants} onSave={onSave} onClose={vi.fn()} />)

    const create = screen.getByRole('button', { name: '创建' })
    expect(create).toBeDisabled()
    const name = screen.getByLabelText('集名称')
    expect(name).toHaveFocus()
    await user.type(name, '写作{Enter}')

    expect(onSave).toHaveBeenCalledWith('写作', '', null, null)
  })

  it('saves color, prompt and default assistant, and submits with Ctrl+Enter from the prompt', async () => {
    const user = userEvent.setup()
    const onSave = vi.fn()
    render(<SetDialog assistants={assistants} onSave={onSave} onClose={vi.fn()} />)

    await user.type(screen.getByLabelText('集名称'), '翻译')
    await user.click(screen.getAllByRole('radio')[2])
    await user.click(screen.getByRole('button', { name: '默认助手' }))
    await user.click(await screen.findByRole('option', { name: '写作助手' }))
    await user.type(screen.getByLabelText(/系统提示词/), '译成中文{Control>}{Enter}{/Control}')

    expect(onSave).toHaveBeenCalledWith('翻译', '译成中文', 'writer', '#2f6ff0')
  })

  it('closes from the close button without saving', async () => {
    const user = userEvent.setup()
    const onSave = vi.fn()
    const onClose = vi.fn()
    render(<SetDialog assistants={assistants} onSave={onSave} onClose={onClose} />)
    await user.click(screen.getAllByRole('button', { name: '取消' })[0])
    expect(onClose).toHaveBeenCalledOnce()
    expect(onSave).not.toHaveBeenCalled()
  })
})
