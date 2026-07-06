import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { Box } from 'lucide-react'
import { Button, IconButton } from './Button'

describe('Button', () => {
  it('renders base kv-btn class by default with type=button', () => {
    render(<Button>保存</Button>)
    const btn = screen.getByRole('button', { name: '保存' })
    expect(btn).toHaveClass('kv-btn')
    expect(btn).toHaveAttribute('type', 'button')
    expect(btn.className).not.toMatch(/\b(primary|accent|ghost|danger|sm)\b/)
  })

  it('maps variant and size to kv-btn modifier classes', () => {
    render(
      <Button variant="primary" size="sm">
        发送
      </Button>,
    )
    const btn = screen.getByRole('button', { name: '发送' })
    expect(btn).toHaveClass('kv-btn', 'primary', 'sm')
  })

  it('appends custom className and forwards native props', async () => {
    const user = userEvent.setup()
    const onClick = vi.fn()
    render(
      <Button variant="ghost" className="extra" onClick={onClick} disabled={false}>
        取消
      </Button>,
    )
    const btn = screen.getByRole('button', { name: '取消' })
    expect(btn).toHaveClass('kv-btn', 'ghost', 'extra')
    await user.click(btn)
    expect(onClick).toHaveBeenCalledOnce()
  })
})

describe('IconButton', () => {
  it('defaults to sm square and uses label for aria-label + title', () => {
    render(
      <IconButton label="删除">
        <Box />
      </IconButton>,
    )
    const btn = screen.getByRole('button', { name: '删除' })
    expect(btn).toHaveClass('kv-icon-btn', 'sm')
    expect(btn).not.toHaveClass('circle')
    expect(btn).toHaveAttribute('title', '删除')
  })

  it('maps size, shape and variant to modifier classes', () => {
    render(
      <IconButton label="关闭" size="lg" shape="circle" variant="danger">
        <Box />
      </IconButton>,
    )
    const btn = screen.getByRole('button', { name: '关闭' })
    expect(btn).toHaveClass('kv-icon-btn', 'lg', 'circle', 'danger')
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
