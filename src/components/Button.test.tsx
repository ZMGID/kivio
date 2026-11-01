import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { Box } from 'lucide-react'
import { Button, IconButton } from './Button'

describe('Button', () => {
  it('defaults to type=button', () => {
    render(<Button>保存</Button>)
    const btn = screen.getByRole('button', { name: '保存' })
    expect(btn).toHaveAttribute('type', 'button')
  })
})

describe('IconButton', () => {
  it('uses the label for the accessible name and title', () => {
    render(
      <IconButton label="删除">
        <Box />
      </IconButton>,
    )
    const btn = screen.getByRole('button', { name: '删除' })
    expect(btn).toHaveAttribute('title', '删除')
  })

  it('prefers explicit title over label when provided', () => {
    render(
      <IconButton label="复制" title="复制到剪贴板">
        <Box />
      </IconButton>,
    )
    const btn = screen.getByRole('button', { name: '复制' })
    expect(btn).toHaveAttribute('title', '复制到剪贴板')
  })
})
