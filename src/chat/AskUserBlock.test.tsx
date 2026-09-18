import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { AskUserBlock } from './AskUserBlock'
import { AsyncQuestionsContext } from './asyncQuestionsContext'
import { api } from '../api/tauri'
import mcpContract from './fixtures/ask-user-mcp-contract.json'
import type { ToolCallRecord } from './types'

/** 待答的问用户卡片。`variant="docked"` 是吊在输入框上方的那张（用户在这里作答）；
 *  消息流里的 inline 变体只剩一行痕迹，那条契约在 ToolCallBlock.test 里。 */
function askUserCall(askUser: Record<string, unknown>): ToolCallRecord {
  return {
    id: 'tool-1',
    toolCallId: 'tool-1',
    toolName: 'AskUserQuestion',
    source: 'external_cli',
    status: 'running',
    structured_content: { askUser },
  }
}

const RETRY_QUESTION = {
  id: '0',
  prompt: '用哪种方式重试？',
  options: [
    { id: '0', label: '指数退避', description: '首次 200ms，每次翻倍' },
    { id: '1', label: '立即重试' },
    { id: '2', label: '不重试' },
  ],
  allow_multiple: false,
  allow_custom: true,
}

describe('AskUserBlock', () => {
  afterEach(() => vi.restoreAllMocks())

  it('round-trips the Rust MCP contract fixture through the form without losing optional or typed values', async () => {
    const submit = vi.spyOn(api, 'chatSubmitUserChoice').mockResolvedValue(undefined)
    render(<AskUserBlock variant="docked" toolCall={askUserCall({
      phase: 'awaiting', questions: mcpContract.expectedQuestions, answers: {},
    })} />)
    fireEvent.click(screen.getByRole('option', { name: /否/ }))
    expect(screen.getByText('environment')).toBeInTheDocument()
    expect(screen.getByText('提交')).toBeDisabled()
    fireEvent.click(screen.getByLabelText('下一题'))
    fireEvent.change(screen.getByPlaceholderText('自己写一个…'), { target: { value: 'draft' } })
    fireEvent.change(screen.getByPlaceholderText('自己写一个…'), { target: { value: '' } })
    fireEvent.click(screen.getByLabelText('下一题'))
    fireEvent.change(screen.getByPlaceholderText('自己写一个…'), { target: { value: '0' } })
    fireEvent.click(screen.getByText('提交'))
    await waitFor(() => expect(submit).toHaveBeenCalledWith('tool-1', mcpContract.answers, false))
  })

  it('renders a schema enum with only one legal choice', async () => {
    const submit = vi.spyOn(api, 'chatSubmitUserChoice').mockResolvedValue(undefined)
    render(<AskUserBlock variant="docked" toolCall={askUserCall({
      phase: 'awaiting', questions: [{
        id: 'mode', prompt: '工作模式', options: [{ id: '0', label: '只读' }], required: true,
        valueSchema: { type: 'string', enum: ['readonly'] },
      }], answers: {},
    })} />)
    fireEvent.click(screen.getByRole('option', { name: /只读/ }))
    await waitFor(() => expect(submit).toHaveBeenCalledWith('tool-1', {
      mode: { selected_option_ids: ['0'], custom_text: null },
    }, false))
  })

  it('preserves a rejected draft for correction and submits zero and false without treating them as missing', async () => {
    const submit = vi.spyOn(api, 'chatSubmitUserChoice')
      .mockRejectedValueOnce(new Error('count: expected a finite number'))
      .mockResolvedValue(undefined)
    const resolved = vi.fn()
    const payload = {
      phase: 'awaiting', questions: [
        { id: 'count', prompt: '数量', options: [], allow_custom: true, required: true, valueSchema: { type: 'number' } },
        { id: 'enabled', prompt: '是否启用', required: true, options: [{ id: 'true', label: '是' }, { id: 'false', label: '否' }], valueSchema: { type: 'boolean' } },
      ], answers: {},
    }
    const { rerender } = render(<AskUserBlock variant="docked" toolCall={askUserCall(payload)} onResolved={resolved} />)
    fireEvent.change(screen.getByPlaceholderText('自己写一个…'), { target: { value: 'invalid' } })
    fireEvent.click(screen.getByLabelText('下一题'))
    fireEvent.click(screen.getByText('否'))
    fireEvent.click(screen.getByText('提交'))
    await screen.findByText('count: expected a finite number')
    expect(resolved).not.toHaveBeenCalled()
    rerender(<AskUserBlock variant="docked" toolCall={askUserCall({ ...payload })} onResolved={resolved} />)
    expect(screen.getAllByRole('option')[1]).toHaveAttribute('aria-selected', 'true')
    fireEvent.click(screen.getByLabelText('上一题'))
    expect(screen.getByPlaceholderText('自己写一个…')).toHaveValue('invalid')
    fireEvent.change(screen.getByPlaceholderText('自己写一个…'), { target: { value: '0' } })
    fireEvent.click(screen.getByText('提交'))
    await waitFor(() => expect(submit).toHaveBeenLastCalledWith('tool-1', {
      count: { selected_option_ids: [], custom_text: '0' },
      enabled: { selected_option_ids: ['false'], custom_text: null },
    }, false))
    expect(resolved).toHaveBeenCalledOnce()
  })

  it('keeps historical questions without required mandatory', () => {
    render(<AskUserBlock variant="docked" toolCall={askUserCall({
      phase: 'awaiting', questions: [RETRY_QUESTION, { ...RETRY_QUESTION, id: 'next' }], answers: {},
    })} />)
    expect(screen.getByLabelText('下一题')).toBeDisabled()
    expect(screen.getByText('提交')).toBeDisabled()
  })

  it('sends only one response when an answer and cancellation arrive in the same render batch', async () => {
    let finish: (() => void) | undefined
    const submit = vi.spyOn(api, 'chatSubmitUserChoice').mockImplementation(() => new Promise<void>((resolve) => { finish = resolve }))
    render(<AskUserBlock variant="docked" toolCall={askUserCall({
      phase: 'awaiting', questions: [RETRY_QUESTION], answers: {},
    })} />)
    act(() => {
      fireEvent.click(screen.getAllByRole('option')[0])
      fireEvent.click(screen.getByLabelText('跳过这次询问'))
    })
    const responseCount = submit.mock.calls.length
    await act(async () => finish?.())
    expect(responseCount).toBe(1)
  })

  it('distinguishes an explicitly edited empty string from an omitted optional string', async () => {
    const submit = vi.spyOn(api, 'chatSubmitUserChoice').mockResolvedValue(undefined)
    render(<AskUserBlock variant="docked" toolCall={askUserCall({
      phase: 'awaiting', questions: [{
        id: 'note', prompt: '补充说明', options: [], allow_custom: true, required: false,
        value_schema: { type: 'string' },
      }], answers: {},
    })} />)
    const input = screen.getByPlaceholderText('自己写一个…')
    fireEvent.change(input, { target: { value: 'draft' } })
    fireEvent.change(input, { target: { value: '' } })
    fireEvent.click(screen.getByText('提交'))
    await waitFor(() => expect(submit).toHaveBeenCalledWith('tool-1', {
      note: { selected_option_ids: [], custom_text: '' },
    }, false))
    submit.mockRestore()
  })

  it('explicitly accepts an all-optional form with an empty object without applying schema defaults', async () => {
    const submit = vi.spyOn(api, 'chatSubmitUserChoice').mockResolvedValue(undefined)
    render(<AskUserBlock variant="docked" toolCall={askUserCall({
      phase: 'awaiting',
      questions: [{
        id: 'note', prompt: '补充说明', options: [], allow_custom: true, required: false,
        valueSchema: { type: 'string', default: '自动填入' },
      }],
      answers: {},
    })} />)
    expect(screen.getByPlaceholderText('自己写一个…')).toHaveValue('')
    expect(submit).not.toHaveBeenCalled()
    fireEvent.click(screen.getByText('提交'))
    await waitFor(() => expect(submit).toHaveBeenCalledWith('tool-1', {}, false))
    submit.mockRestore()
  })

  it('allows skipping an optional middle question and omits it from the submitted answers', async () => {
    const submit = vi.spyOn(api, 'chatSubmitUserChoice').mockResolvedValue(undefined)
    render(<AskUserBlock variant="docked" toolCall={askUserCall({
      phase: 'awaiting',
      questions: [
        RETRY_QUESTION,
        { id: 'note', prompt: '补充说明', options: [], allow_custom: true, required: false },
        { id: 'last', prompt: '确认方案', options: RETRY_QUESTION.options },
      ],
      answers: {},
    })} />)
    fireEvent.click(screen.getAllByRole('option')[0])
    expect(screen.getByLabelText('下一题')).toBeEnabled()
    expect(screen.getByText('提交')).toBeDisabled()
    fireEvent.click(screen.getByLabelText('下一题'))
    fireEvent.click(screen.getAllByRole('option')[1])
    fireEvent.click(screen.getByText('提交'))
    await waitFor(() => expect(submit).toHaveBeenCalledWith('tool-1', {
      '0': { selected_option_ids: ['0'], custom_text: null },
      last: { selected_option_ids: ['1'], custom_text: null },
    }, false))
    submit.mockRestore()
  })

  it('answers asynchronous inline cards through ordinary messages without stealing focus', async () => {
    const reply = vi.fn().mockResolvedValue(undefined)
    render(<AsyncQuestionsContext.Provider value={{ closedIds: new Set(), reply }}>
      <input aria-label="composer" autoFocus />
      <AskUserBlock toolCall={askUserCall({
        async: true, phase: 'awaiting', questions: [RETRY_QUESTION], answers: {},
      })} />
    </AsyncQuestionsContext.Provider>)
    expect(screen.getByLabelText('composer')).toHaveFocus()
    fireEvent.click(screen.getAllByRole('option')[1])
    await waitFor(() => expect(reply).toHaveBeenCalledWith('tool-1', '用哪种方式重试？\n立即重试'))
  })

  it('supports free text and skipping async questions, and closes superseded cards', async () => {
    const reply = vi.fn().mockResolvedValue(undefined)
    const call = askUserCall({ async: true, phase: 'awaiting', questions: [{
      id: '0', prompt: '还有补充吗？', options: [], allow_custom: true,
    }], answers: {} })
    const { rerender } = render(<AsyncQuestionsContext.Provider value={{ closedIds: new Set(), reply }}>
      <AskUserBlock toolCall={call} />
    </AsyncQuestionsContext.Provider>)
    const input = screen.getByPlaceholderText('自己写一个…')
    fireEvent.change(input, { target: { value: '保留兼容性' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    await waitFor(() => expect(reply).toHaveBeenCalledWith('tool-1', '还有补充吗？\n保留兼容性'))
    fireEvent.click(screen.getByLabelText('跳过这次询问'))
    await waitFor(() => expect(reply).toHaveBeenCalledWith('tool-1', null))
    rerender(<AsyncQuestionsContext.Provider value={{ closedIds: new Set(['tool-1']), reply }}>
      <AskUserBlock toolCall={call} />
    </AsyncQuestionsContext.Provider>)
    expect(screen.queryByPlaceholderText('自己写一个…')).not.toBeInTheDocument()
    expect(screen.getByText('此问题已收起')).toBeInTheDocument()
  })

  it('renders the question as the title with numbered options', () => {
    render(<AskUserBlock variant="docked" toolCall={askUserCall({
      phase: 'awaiting',
      questions: [RETRY_QUESTION],
      answers: {},
    })} />)
    expect(screen.getByText('用哪种方式重试？')).toBeInTheDocument()
    expect(screen.getAllByRole('option')).toHaveLength(3)
    expect(screen.getByText('指数退避')).toBeInTheDocument()
    expect(screen.getByText('首次 200ms，每次翻倍')).toBeInTheDocument()
    // 单题：不出「1/1」也不出翻页键，一道题的进度条是纯噪声。
    expect(screen.queryByText('1/1')).not.toBeInTheDocument()
    expect(screen.queryByLabelText('上一题')).not.toBeInTheDocument()
    expect(screen.queryByLabelText('下一题')).not.toBeInTheDocument()
    // 单题单选点一下就是答案，不该再出现「提交」。
    expect(screen.queryByText('提交')).not.toBeInTheDocument()
  })

  it('shows the pager and a submit button only when there are several questions', () => {
    render(<AskUserBlock variant="docked" toolCall={askUserCall({
      phase: 'awaiting',
      questions: [
        RETRY_QUESTION,
        { id: '1', prompt: '要补测试吗？', options: [{ id: '0', label: '要' }, { id: '1', label: '不要' }], allow_multiple: false, allow_custom: false },
      ],
      answers: {},
    })} />)
    expect(screen.getByText('1/2')).toBeInTheDocument()
    expect(screen.getByLabelText('下一题')).toBeInTheDocument()
    expect(screen.getByText('提交')).toBeInTheDocument()
  })

  // 多选时同一批次连点两项：React 会把两次点击合成一次渲染，若从 state 里读草稿，
  // 第二下会拿着旧值把第一下覆盖掉（实测只剩后一项）。
  it('keeps both picks when two multi-select options are clicked in one batch', () => {
    render(<AskUserBlock variant="docked" toolCall={askUserCall({
      phase: 'awaiting',
      questions: [{
        id: '0',
        prompt: '哪些模块要一起改？',
        options: [{ id: '0', label: '运行时' }, { id: '1', label: '用量面板' }, { id: '2', label: '右侧栏' }],
        allow_multiple: true,
        allow_custom: false,
      }],
      answers: {},
    })} />)
    const options = screen.getAllByRole('option')
    // 包在同一个 act 里才是「一次渲染两次点击」—— 单独 fireEvent 会各自 flush，复现不出来。
    act(() => {
      fireEvent.click(options[1])
      fireEvent.click(options[2])
    })
    expect(screen.getAllByRole('option')[1]).toHaveAttribute('aria-selected', 'true')
    expect(screen.getAllByRole('option')[2]).toHaveAttribute('aria-selected', 'true')
  })

  it('renders a free-text question when there are no options', () => {
    render(<AskUserBlock variant="docked" toolCall={askUserCall({
      phase: 'awaiting',
      questions: [{
        id: 'note',
        prompt: '还有补充吗？',
        options: [],
        allow_multiple: false,
        allow_custom: true,
      }],
      answers: {},
    })} />)
    expect(screen.getByText('还有补充吗？')).toBeInTheDocument()
    expect(screen.queryByRole('option')).not.toBeInTheDocument()
    expect(screen.getByPlaceholderText('自己写一个…')).toBeInTheDocument()
  })

  it('renders answers read-only once the phase is answered', () => {
    render(<AskUserBlock variant="docked" toolCall={askUserCall({
      phase: 'answered',
      questions: [RETRY_QUESTION],
      answers: { '0': { selected_option_ids: ['1'], custom_text: null } },
    })} />)
    expect(screen.getByText('已回答')).toBeInTheDocument()
    expect(screen.getByText('立即重试')).toBeInTheDocument()
    expect(screen.queryByRole('option')).not.toBeInTheDocument()
  })
})
