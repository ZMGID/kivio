import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { ToolCallBlock } from './ToolCallBlock'
import type { ToolCallRecord } from './types'

function buildToolCall(overrides: Partial<ToolCallRecord> = {}): ToolCallRecord {
  return {
    id: 'tool-1',
    toolName: 'read_file',
    status: 'success',
    result_preview: 'file contents loaded',
    ...overrides,
  }
}

describe('ToolCallBlock', () => {
  it('renders a localized verb + basename target, dropping status/source/duration', () => {
    render(<ToolCallBlock toolCall={buildToolCall({ arguments: { path: 'src/a/README.md' } })} />)
    const button = screen.getByRole('button', { name: /讀取/ })
    // Cursor-style row: 動詞 + 目標（檔名 basename）
    expect(within(button).getByText('讀取')).toBeInTheDocument()
    expect(within(button).getByText('README.md')).toBeInTheDocument()
    // 已刪除的後綴 / 全路徑不再出現在折疊列
    expect(within(button).queryByText(/已完成/)).not.toBeInTheDocument()
    expect(within(button).queryByText(/Kivio/)).not.toBeInTheDocument()
    expect(within(button).queryByText(/file contents loaded/)).not.toBeInTheDocument()
    expect(within(button).queryByText(/src\/a/)).not.toBeInTheDocument()
  })

  it('shows the real read line range from structured content', () => {
    render(
      <ToolCallBlock
        toolCall={buildToolCall({
          toolName: 'read',
          arguments: { path: 'src/chat/Lens.tsx' },
          structured_content: { path: 'src/chat/Lens.tsx', start_line: 1880, end_line: 1939 },
        })}
      />,
    )
    const button = screen.getByRole('button', { name: /讀取/ })
    expect(within(button).getByText('Lens.tsx L1880-1939')).toBeInTheDocument()
  })

  it('keeps the error out of the collapsed row and shows it (not red) in the expanded detail', async () => {
    const user = userEvent.setup()
    render(
      <ToolCallBlock
        toolCall={buildToolCall({
          status: 'error',
          error: 'permission denied',
        })}
      />,
    )
    const button = screen.getByRole('button', { name: /讀取/ })
    expect(within(button).queryByText(/permission denied/)).not.toBeInTheDocument()
    await user.click(button)
    const detail = screen.getByText(/permission denied/)
    expect(detail).toBeInTheDocument()
    // 错误不再标红
    expect(detail.className).not.toContain('text-red-500')
  })

  it('expands details when clicked', async () => {
    const user = userEvent.setup()
    render(
      <ToolCallBlock
        toolCall={buildToolCall({
          arguments: { path: 'README.md' },
        })}
        defaultOpen={false}
      />,
    )
    await user.click(screen.getByRole('button', { name: /讀取/ }))
    expect(screen.getAllByText(/README\.md/).length).toBeGreaterThan(0)
    expect(screen.getByText('引數')).toBeInTheDocument()
  })

  it('uses the search pattern as the grep target', () => {
    render(
      <ToolCallBlock
        toolCall={buildToolCall({
          toolName: 'grep',
          result_preview: '',
          arguments: {
            query: 'ClaudeAgentClient',
            path: 'packages/server/src/server/agent/providers/claude/agent.ts',
          },
        })}
      />,
    )
    const button = screen.getByRole('button', { name: /搜尋/ })
    expect(within(button).getByText('搜尋')).toBeInTheDocument()
    expect(within(button).getByText('ClaudeAgentClient')).toBeInTheDocument()
    // 目標只取 pattern，不再把 scope 塞進折疊列
    expect(within(button).queryByText(/agent\.ts/)).not.toBeInTheDocument()
  })

  it('renders glob as localized verb + pattern + directory', () => {
    render(
      <ToolCallBlock
        toolCall={buildToolCall({
          toolName: 'glob',
          result_preview: '',
          arguments: { pattern: '**/*overlay*', path: 'src/lens' },
        })}
      />,
    )
    const button = screen.getByRole('button', { name: /比對/ })
    expect(within(button).getByText('比對')).toBeInTheDocument()
    expect(within(button).getByText('**/*overlay* 於 lens')).toBeInTheDocument()
  })

  it('falls back to stored grep argument preview when parsed arguments are unavailable', () => {
    render(
      <ToolCallBlock
        toolCall={buildToolCall({
          toolName: 'grep',
          result_preview: '',
          arguments: '{"query":',
          argumentPreview: '正在生成工具引數…',
          argumentsPreview: '正在生成工具引數…',
        })}
      />,
    )
    const button = screen.getByRole('button', { name: /搜尋/ })
    expect(within(button).getByText(/正在生成工具引數/)).toBeInTheDocument()
    expect(within(button).queryByText(/搜尋文字/)).not.toBeInTheDocument()
  })

  it('shows the command as the command target', () => {
    render(
      <ToolCallBlock
        toolCall={buildToolCall({
          toolName: 'run_command',
          result_preview: 'exit_code: 0',
          arguments: { command: 'npm test' },
        })}
      />,
    )
    const button = screen.getByRole('button', { name: /執行/ })
    expect(within(button).getByText('執行')).toBeInTheDocument()
    expect(within(button).getByText('npm test')).toBeInTheDocument()
    expect(within(button).queryByText(/exit_code/)).not.toBeInTheDocument()
  })
})
