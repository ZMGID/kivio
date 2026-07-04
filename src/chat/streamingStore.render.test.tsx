import { memo, useRef } from 'react'
import { act, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { MessageList } from './MessageList'
import {
  getCoarse,
  patchSnapshot,
  reset,
  setCoarse,
  setSnapshot,
} from './streamingStore'
import { createEmptyStreamSnapshot } from './conversationRuns'
import type { ConversationStreamSnapshot } from './conversationRuns'
import type { ChatMessage } from './types'

// 真實整合：掛載真 MessageList（訂閱真 streamingStore），按 Chat 各 helper 的呼叫方式驅動 store，
// 驗證「流式更新只重渲訂閱者、不波及兄弟節點」這一核心收益，以及各 helper→store 對映的渲染結果。

function snapWith(partial: Partial<ConversationStreamSnapshot>): ConversationStreamSnapshot {
  return { ...createEmptyStreamSnapshot(), ...partial }
}

// MessageList 現在用 virtua 虛擬化：視口測量 + 可見區間計算發生在 mount 後的一個微任務，
// 故斷言渲染結果前需讓 React 把這次非同步更新刷出來。
async function flush() {
  await act(async () => {
    await Promise.resolve()
  })
}

afterEach(() => {
  act(() => {
    reset()
    setCoarse({ streaming: false, streamFrozen: false, cancelling: false, streamError: '' })
  })
})

// 不訂閱 store 的兄弟節點，記錄自身渲染次數。
let siblingRenders = 0
const Sibling = memo(function Sibling() {
  const count = useRef(0)
  count.current += 1
  siblingRenders = count.current
  return <div data-testid="sibling">sibling</div>
})

function mountList() {
  return render(
    <>
      <MessageList messages={[]} conversationId="c1" />
      <Sibling />
    </>,
  )
}

function message(id: number): ChatMessage {
  return {
    id: `m-${id}`,
    role: id % 2 === 0 ? 'user' : 'assistant',
    content: `message ${id}`,
    timestamp: id,
  }
}

describe('MessageList ← streamingStore 整合', () => {
  it('does not render a detached global agent plan row', async () => {
    const onExecute = vi.fn()
    render(
      <MessageList
        conversationId="c-plan"
        messages={[{
          id: 'msg-plan',
          role: 'assistant',
          content: '1. Read code\n2. Implement',
          timestamp: 1,
        }]}
        agentPlanState={{ mode: 'plan', status: 'draft', plan: '1. Read code\n2. Implement', updated_at: 1 }}
        onExecuteAgentPlan={onExecute}
      />,
    )
    await flush()

    expect(document.querySelector('[data-chat-message-list-item="plan"]')).not.toBeInTheDocument()
    const button = screen.getByRole('button', { name: '執行這條計畫' })
    expect(button).toBeInTheDocument()
    await act(async () => {
      button.click()
    })
    expect(onExecute).toHaveBeenCalledWith('msg-plan')
  })

  it('does not attach a legacy agent plan row to non-plan text', async () => {
    render(
      <MessageList
        conversationId="c-plan-fragment"
        messages={[{
          id: 'msg-plan-fragment',
          role: 'assistant',
          content: '沒問題！積萌,',
          timestamp: 1,
        }]}
        agentPlanState={{ mode: 'plan', status: 'draft', plan: '沒問題！積萌,', updated_at: 1 }}
        onExecuteAgentPlan={() => {}}
      />,
    )
    await flush()

    expect(screen.queryByText('計畫草案')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '執行這條計畫' })).not.toBeInTheDocument()
  })

  it('applyStreamSnapshotToState 等價：內容快照 + coarse streaming → 渲染流式預覽文字', async () => {
    siblingRenders = 0
    mountList()
    expect(siblingRenders).toBe(1)

    // 模擬 applyStreamSnapshotToState：setSnapshot(snapshot) + setCoarse({streaming:true})
    act(() => {
      setSnapshot(snapWith({ content: 'hello streaming world', streaming: true }))
      setCoarse({ streaming: true, cancelling: false })
    })
    await flush()
    expect(screen.getByText(/hello streaming world/)).toBeInTheDocument()
  })

  it('流式逐幀更新只重渲 MessageList，不波及未訂閱的兄弟節點', async () => {
    siblingRenders = 0
    mountList()
    const baseline = siblingRenders // 1

    act(() => setCoarse({ streaming: true }))
    // 連續多幀內容更新（模擬 RAF 每幀 setSnapshot）
    for (let i = 0; i < 5; i++) {
      act(() => setSnapshot(snapWith({ content: `frame ${i}`, streaming: true })))
    }
    await flush()
    expect(screen.getByText(/frame 4/)).toBeInTheDocument()
    // 兄弟節點渲染次數不變 —— 證明 store 把更新隔離到訂閱者。
    expect(siblingRenders).toBe(baseline)
  })

  it('cancelCurrentRunLocally 等價：coarse streaming:false+frozen:true + patchSnapshot 凍結保留文字', async () => {
    mountList()
    act(() => {
      setSnapshot(snapWith({ content: 'partial answer', streaming: true }))
      setCoarse({ streaming: true })
    })
    await flush()
    expect(screen.getByText(/partial answer/)).toBeInTheDocument()

    act(() => {
      setCoarse({ streaming: false, streamFrozen: true })
      patchSnapshot({ reasoningStreaming: false })
    })
    await flush()
    // 凍結態下已生成文字仍在（streamFrozen 讓預覽繼續渲染）。
    expect(screen.getByText(/partial answer/)).toBeInTheDocument()
    expect(getCoarse().streamFrozen).toBe(true)
  })

  it('reset（clearStreamingPreview 等價）清掉預覽但保留 streamError', async () => {
    mountList()
    act(() => {
      setSnapshot(snapWith({ content: 'to be cleared', streaming: true }))
      setCoarse({ streaming: true, streamError: 'boom' })
    })
    await flush()
    expect(screen.getByText(/to be cleared/)).toBeInTheDocument()

    act(() => reset())
    await flush()
    expect(screen.queryByText(/to be cleared/)).not.toBeInTheDocument()
    // streamError 不被 reset 清除（與原 clearStreamingPreview 語義一致），錯誤文案仍展示。
    expect(screen.getByText('boom')).toBeInTheDocument()
  })

  it('長列表只掛載可見視窗，而不是把所有歷史訊息留在 DOM', async () => {
    const messages = Array.from({ length: 100 }, (_, index) => message(index))
    render(<MessageList messages={messages} conversationId="long-c1" />)
    await flush()

    const mountedMessages = document.querySelectorAll('[data-chat-message-list-item="message"]')
    expect(mountedMessages.length).toBeGreaterThan(0)
    expect(mountedMessages.length).toBeLessThan(messages.length)
  })
})
