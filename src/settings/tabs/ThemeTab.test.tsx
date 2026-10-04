import { useState } from 'react'
import { fireEvent, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { ThemeTab } from './ThemeTab'
import { makeSettings } from './testFixtures'
import { BUILTIN_THEMES } from '../../theme/theme'
import type { ThemeDefinition } from '../../theme/types'
import type { Settings } from '../../api/tauri'

function Harness({ failFirst = false }: { failFirst?: boolean }) {
  const [settings, setSettings] = useState(makeSettings())
  const [draft, setDraft] = useState<ThemeDefinition | null>(null)
  const [failed, setFailed] = useState(false)
  return <ThemeTab settings={settings} lang="zh" draft={draft} onDraftChange={setDraft}
    onCommit={async (update: (current: Settings) => Settings) => {
      if (failFirst && !failed) { setFailed(true); throw new Error('Disk unavailable') }
      setSettings(current => update(current))
    }} />
}

describe('ThemeTab', () => {
  it('retains the edited palette after a failed save and lets the user retry', async () => {
    const user = userEvent.setup()
    render(<Harness failFirst />)
    await user.click(screen.getByRole('button', { name: '复制为自定义主题' }))
    const name = screen.getByRole('textbox', { name: '主题名称' })
    await user.clear(name)
    await user.type(name, 'My theme')
    const color = screen.getByRole('textbox', { name: 'light.surface' })
    await user.clear(color)
    await user.type(color, '#abcdef')
    await user.click(screen.getByRole('button', { name: '保存主题' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Disk unavailable')
    expect(screen.getByRole('textbox', { name: 'light.surface' })).toHaveValue('#abcdef')
    await user.click(screen.getByRole('button', { name: '保存主题' }))
    expect(await screen.findByRole('radio', { name: /My theme/ })).toBeEnabled()
    expect(screen.queryByRole('textbox', { name: '主题名称' })).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '应用主题' }))
    expect(await screen.findByRole('radio', { name: /My theme/ })).toHaveTextContent('正在使用')
    expect(BUILTIN_THEMES[0].light.surface).not.toBe('#abcdef')
  })

  it('rejects invalid colors without replacing the draft or allowing save', async () => {
    const user = userEvent.setup()
    render(<Harness />)
    await user.click(screen.getByRole('button', { name: '复制为自定义主题' }))
    const color = screen.getByRole('textbox', { name: 'light.surface' })
    await user.clear(color)
    await user.type(color, '#bad')
    expect(screen.getByRole('button', { name: '保存主题' })).toBeDisabled()
    expect(color).toHaveAttribute('aria-invalid', 'true')
    await user.click(screen.getByRole('button', { name: '取消编辑' }))
    expect(screen.queryByRole('textbox', { name: 'light.surface' })).not.toBeInTheDocument()
    expect(screen.getByRole('radio', { name: /中性/ })).toBeEnabled()
  })

  it('lets keyboard selection preview a theme without changing the active theme', async () => {
    const user = userEvent.setup()
    render(<Harness />)
    screen.getByRole('radio', { name: /中性/ }).focus()
    await user.keyboard('{ArrowRight}')
    expect(screen.getByRole('radio', { name: /暖白/ })).toHaveFocus()
    expect(screen.getByRole('radio', { name: /暖白/ })).toBeChecked()
    expect(screen.getByRole('radio', { name: /中性/ })).toHaveTextContent('正在使用')
    expect(screen.getByRole('button', { name: '应用主题' })).toBeEnabled()
    await user.keyboard('{End}{ArrowRight}')
    expect(screen.getByRole('radio', { name: /中性/ })).toHaveFocus()
    expect(screen.getByRole('radio', { name: /中性/ })).toBeChecked()
  })

  it('synchronizes the picker and hex input, retaining invalid colors across palette switches', async () => {
    const user = userEvent.setup()
    render(<Harness />)
    await user.click(screen.getByRole('button', { name: '复制为自定义主题' }))
    fireEvent.change(screen.getByLabelText('light.surface.picker'), { target: { value: '#123456' } })
    expect(screen.getByRole('textbox', { name: 'light.surface' })).toHaveValue('#123456')
    const modes = within(screen.getByRole('group', { name: '编辑色板' }))
    await user.click(modes.getByRole('button', { name: '深色' }))
    const dark = screen.getByRole('textbox', { name: 'dark.surface' })
    await user.clear(dark)
    await user.type(dark, '#bad')
    await user.click(modes.getByRole('button', { name: '浅色' }))
    expect(screen.getByRole('textbox', { name: 'light.surface' })).toHaveValue('#123456')
    expect(screen.getByRole('button', { name: '保存主题' })).toBeDisabled()
    expect(screen.getByRole('alert')).toHaveTextContent('另一套色板')
    await user.click(modes.getByRole('button', { name: /深色/ }))
    expect(screen.getByRole('textbox', { name: 'dark.surface' })).toHaveValue('#bad')
  })
})
