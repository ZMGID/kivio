import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { ChatMarkdown } from './ChatMarkdown'
import { MarkdownStreamingContext } from './markdownStreaming'
import {
  beginConversationTransition,
  cancelConversationTransition,
  getConversationTransitionSnapshot,
} from './conversationTransitionStore'
import {
  beginMessageNavigationHydrate,
  beginStreamSettleEagerHydrate,
  endMessageNavigationHydrate,
  resetMessageNavigationStore,
} from './messageNavigationStore'

describe('ChatMarkdown streaming stability', () => {
  it('preserves links and loaded images when artifacts update during streaming and commit', async () => {
    const content = '[source](https://example.com)\n\n![chart](chart.png)'
    const artifact = { name: 'chart.png', mimeType: 'image/png', dataUrl: 'data:image/png;base64,AAAA' }
    const view = (streaming: boolean, tail: string) => (
      <MarkdownStreamingContext.Provider value={streaming}>
        <ChatMarkdown content={`${content}\n\n${tail}`} artifacts={[{ ...artifact }]} />
      </MarkdownStreamingContext.Provider>
    )
    const { container, rerender } = render(view(true, 'frame 0'))
    await act(async () => { await Promise.resolve() })
    const link = container.querySelector('a')
    const image = container.querySelector('img')
    expect(link).not.toBeNull()
    expect(image).not.toBeNull()
    for (const streaming of [true, false]) {
      rerender(view(streaming, `frame ${streaming ? 1 : 2}`))
      await act(async () => { await Promise.resolve() })
      expect(container.querySelector('a')).toBe(link)
      expect(container.querySelector('img')).toBe(image)
    }
  })

  it.each([true, false])('updates revised text without remounting unchanged blocks (streaming=%s)', async (streaming) => {
    const view = (tail: string) => (
      <MarkdownStreamingContext.Provider value={streaming}>
        <ChatMarkdown content={`## Stable heading\n\n${tail}`} />
      </MarkdownStreamingContext.Provider>
    )
    const { container, rerender } = render(view('frame 0'))
    await act(async () => { await Promise.resolve() })
    const heading = container.querySelector('h2')
    expect(heading).not.toBeNull()
    for (const tail of ['frame 1', 'replacement **body**', 'short', '', 'final body']) {
      rerender(view(tail))
      await act(async () => { await Promise.resolve() })
      expect(container.querySelector('h2')).toBe(heading)
      expect(container.textContent).toContain(tail.replaceAll('**', ''))
      expect(container.textContent).not.toContain('frame 0')
    }
  })

  afterEach(() => {
    const { requestId } = getConversationTransitionSnapshot()
    if (requestId > 0) cancelConversationTransition(requestId)
    resetMessageNavigationStore()
  })

  it('流式中未闭合加粗由 Streamdown parseIncomplete 补全', async () => {
    const { container, rerender } = render(
      <MarkdownStreamingContext.Provider value={true}>
        <ChatMarkdown content={'前缀 **加粗'} />
      </MarkdownStreamingContext.Provider>,
    )

    // streaming 模式块更新可能走 transition；等一拍再断言。
    await act(async () => {
      await Promise.resolve()
    })
    expect(container.querySelector('[data-streamdown="strong"]')?.textContent).toBe('加粗')
    expect(container.textContent).toContain('前缀')
    expect(container.textContent).not.toContain('**')

    rerender(
      <MarkdownStreamingContext.Provider value={true}>
        <ChatMarkdown content={'前缀 **加粗文字'} />
      </MarkdownStreamingContext.Provider>,
    )
    await act(async () => {
      await Promise.resolve()
    })
    expect(container.querySelector('[data-streamdown="strong"]')?.textContent).toBe('加粗文字')
  })

  it('定稿与历史不把未闭合加粗交给 remend 补全', async () => {
    const { container } = render(<ChatMarkdown content={'前缀 **加粗'} />)
    await act(async () => { await Promise.resolve() })
    expect(container.querySelector('[data-streamdown="strong"]')).toBeNull()
    expect(container.textContent).toContain('前缀')
    expect(container.textContent).toContain('**加粗')
  })

  it('未写完的图片在流式中仍被收起，落库历史保留地址', async () => {
    // remend 1.3 对未闭合图片的处理是整段删掉（`说明` 之后的 `![趋势图](url` 消失）。
    // 定稿必须留下用户还能看见的地址，不能把截断的历史修成少了一段。
    const content = '说明\n\n![趋势图](https://cdn.example/trend.png'
    const view = (streaming: boolean) => (
      <MarkdownStreamingContext.Provider value={streaming}>
        <ChatMarkdown content={content} />
      </MarkdownStreamingContext.Provider>
    )
    const { container, rerender } = render(view(true))
    await act(async () => { await Promise.resolve() })
    expect(container.textContent).toContain('说明')
    expect(container.textContent).not.toContain('https://cdn.example/trend.png')

    rerender(view(false))
    await act(async () => { await Promise.resolve() })
    expect(container.textContent).toContain('https://cdn.example/trend.png')
    expect(container.textContent).toContain('趋势图')
  })

  it('定稿恢复未闭合语法的原文，仅修正受影响的块，不重挂旁边代码', async () => {
    const content = '前缀 **加粗\n\n```ts\nconst stable = 1\n```\n\n```ts\nconst pending = 1'
    const view = (streaming: boolean) => (
      <MarkdownStreamingContext.Provider value={streaming}>
        <ChatMarkdown content={content} />
      </MarkdownStreamingContext.Provider>
    )
    const { container, rerender } = render(view(true))
    await act(async () => { await Promise.resolve() })
    const codes = () => [...container.querySelectorAll('figure pre code')]
    expect(codes()[0]?.textContent).toBe('const stable = 1')
    const stable = codes()[0]

    rerender(view(false))
    await act(async () => { await Promise.resolve() })
    const settled = codes()
    expect(settled.map((node) => node.textContent)).toEqual(['const stable = 1', 'const pending = 1'])
    expect(settled[0]).toBe(stable)
    expect(container.querySelector('[data-streamdown="strong"]')).toBeNull()
    expect(container.textContent).toContain('**加粗')
  })

  it('流式代码块不走 ChatHeavyIsland 延迟 hydrate', async () => {
    const { container } = render(
      <MarkdownStreamingContext.Provider value={true}>
        <ChatMarkdown content={'```ts\nconst x = 1\n```'} />
      </MarkdownStreamingContext.Provider>,
    )

    await act(async () => {
      await Promise.resolve()
    })
    // 外壳与历史气泡同构（同一个 island div，settle 前后几何一致），但流式下首挂即 hydrated。
    const island = container.querySelector('[data-chat-heavy-island="true"]')
    expect(island?.getAttribute('data-chat-heavy-hydrated')).toBe('true')
    expect(container.querySelector('figure pre code')?.textContent).toContain('const x = 1')
  })

  it('历史代码块默认延迟 hydrate，会话打开中则立刻 hydrate', async () => {
    const { container, unmount } = render(
      <ChatMarkdown content={'```ts\nconst x = 1\n```'} />,
    )
    await act(async () => {
      await Promise.resolve()
    })
    const island = container.querySelector('[data-chat-heavy-island="true"]')
    expect(island).not.toBeNull()
    expect(island?.getAttribute('data-chat-heavy-hydrated')).toBe('false')
    unmount()

    beginConversationTransition('conv-open', { messageCount: 20 })
    const opening = render(
      <ChatMarkdown content={'```ts\nconst y = 2\n```'} />,
    )
    await act(async () => {
      await Promise.resolve()
    })
    const openIsland = opening.container.querySelector('[data-chat-heavy-island="true"]')
    expect(openIsland?.getAttribute('data-chat-heavy-hydrated')).toBe('true')
    expect(opening.container.querySelector('figure pre code')?.textContent).toContain('const y = 2')
    opening.unmount()
  })

  it('消息导航 settle 期间历史代码块立刻 hydrate', async () => {
    const generation = beginMessageNavigationHydrate()
    const { container, unmount } = render(
      <ChatMarkdown content={'```ts\nconst z = 3\n```'} />,
    )
    await act(async () => {
      await Promise.resolve()
    })
    const island = container.querySelector('[data-chat-heavy-island="true"]')
    expect(island?.getAttribute('data-chat-heavy-hydrated')).toBe('true')
    expect(container.querySelector('figure pre code')?.textContent).toContain('const z = 3')
    unmount()
    endMessageNavigationHydrate(generation)
  })

  // 生成结束 live → twin 是 streaming 树换 static 树。两棵树的顶层结构必须逐元素一致，
  // 否则根上的 space-y-4 / first-child:mt-0 落点不同，整篇在 settle 那一帧重排（「格式变了一下」）。
  // 已知差异来源：给 Streamdown 传 dir 会让 streaming 模式把每块包进 display:contents div；
  // 流式代码块若不走 DeferredCodeBlock 外壳，twin 会多一层 island div。
  it('streaming 与 static 模式渲染出相同的顶层 DOM 结构', async () => {
    const content = [
      '## 标题', '', '第一段 **加粗**。', '第二行软换行。', '',
      '- 一', '- 二', '  - 嵌套', '', '1. 甲', '2. 乙', '',
      '```ts', 'const x = 1', '```', '',
      '| a | b |', '|---|---|', '| 1 | 2 |', '', '> 引用', '', '结尾。',
    ].join('\n')
    const shape = (container: HTMLElement) => {
      const root = container.querySelector('.chat-markdown [class*="space-y-4"]') as HTMLElement
      return [...root.children].map((el) => `${el.tagName}${el.getAttribute('style') ?? ''}`)
    }

    const live = render(
      <MarkdownStreamingContext.Provider value={true}>
        <ChatMarkdown content={content} />
      </MarkdownStreamingContext.Provider>,
    )
    await act(async () => { await Promise.resolve() })
    const liveShape = shape(live.container)
    live.unmount()

    beginStreamSettleEagerHydrate()
    const settled = render(<ChatMarkdown content={content} />)
    await act(async () => { await Promise.resolve() })
    expect(liveShape.length).toBeGreaterThan(5)
    expect(shape(settled.container)).toEqual(liveShape)
    settled.unmount()
  })

  it('流式结束短窗 eager：历史代码块首挂即 hydrate，避免 180ms 再撑高', async () => {
    beginStreamSettleEagerHydrate()
    const { container, unmount } = render(
      <ChatMarkdown content={'```ts\nconst after = 1\n```'} />,
    )
    await act(async () => {
      await Promise.resolve()
    })
    const island = container.querySelector('[data-chat-heavy-island="true"]')
    expect(island?.getAttribute('data-chat-heavy-hydrated')).toBe('true')
    expect(container.querySelector('figure pre code')?.textContent).toContain('const after = 1')
    unmount()
  })

  it('流式代码保持转义全文和复制，定稿后同一 code 恢复高亮', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    const previousClipboard = navigator.clipboard
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } })
    try {
      const source = 'const label = "<b>hi</b>" & ok'
      const longer = `${source}\nconst next = 2`
      const view = (streaming: boolean, body: string) => (
        <MarkdownStreamingContext.Provider value={streaming}>
          <ChatMarkdown content={`\`\`\`ts\n${body}\n\`\`\``} />
        </MarkdownStreamingContext.Provider>
      )
      const { container, rerender } = render(view(true, source))
      await act(async () => { await Promise.resolve() })

      const code = container.querySelector('figure pre code') as HTMLElement
      expect(code.textContent).toBe(source)
      expect(code.querySelector('span, b')).toBeNull()
      expect(code.innerHTML).toContain('&lt;b&gt;')
      expect(code.innerHTML).toContain('&amp;')

      rerender(view(true, longer))
      await act(async () => { await Promise.resolve() })
      const grown = container.querySelector('figure pre code')
      expect(grown?.textContent).toBe(longer)
      expect(grown?.querySelector('span')).toBeNull()

      const copyButton = screen.getByRole('button', { name: '复制代码' })
      await act(async () => {
        fireEvent.click(copyButton)
      })
      expect(writeText).toHaveBeenCalledWith(longer)

      rerender(view(false, longer))
      await act(async () => { await Promise.resolve() })
      const settled = container.querySelector('figure pre code')
      expect(settled).toBe(grown)
      expect(settled?.textContent).toBe(longer)
      const keyword = settled?.querySelector('span')
      expect(keyword?.textContent).toBe('const')

      writeText.mockClear()
      await act(async () => {
        fireEvent.click(copyButton)
      })
      expect(writeText).toHaveBeenCalledWith(longer)
    } finally {
      Object.defineProperty(navigator, 'clipboard', { configurable: true, value: previousClipboard })
    }
  })
})
