import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { ChatRouteKeepAlive } from './ChatRouteKeepAlive'
import chatStyles from '../styles/chat-01-main.css?inline'

describe('ChatRouteKeepAlive', () => {
  it('页面透明入场时，常驻阅读底色不会随路由消失', () => {
    const stylesheet = document.createElement('style')
    // jsdom 不解析 CSS 自定义属性；只替换色板值，仍加载真实样式规则。
    stylesheet.textContent = chatStyles.split('var(--theme-surface)').join('#faf9f5')
    document.head.append(stylesheet)
    try {
      const { container, rerender } = render(
        <ChatRouteKeepAlive activeKey="conversation">
          <main style={{ opacity: 0 }}>chat entering</main>
        </ChatRouteKeepAlive>,
      )
      const surface = container.firstElementChild!
      for (const route of ['conversation', 'center', 'settings', 'conversation']) {
        rerender(
          <ChatRouteKeepAlive activeKey={route}>
            <main style={{ opacity: 0 }}>{route} entering</main>
          </ChatRouteKeepAlive>,
        )
        expect(container.firstElementChild).toBe(surface)
        expect(getComputedStyle(surface).backgroundColor).toBe('rgb(250, 249, 245)')
      }
    } finally {
      stylesheet.remove()
    }
  })

  it('切换设置页后复用原聊天 DOM 实例', () => {
    const { rerender } = render(
      <ChatRouteKeepAlive activeKey="conversation">
        <main data-testid="chat-pane">chat</main>
      </ChatRouteKeepAlive>,
    )
    const firstPane = document.querySelector('[data-testid="chat-pane"]')
    expect(firstPane).not.toBeNull()

    rerender(
      <ChatRouteKeepAlive activeKey="settings">
        <section data-testid="settings-pane">settings</section>
      </ChatRouteKeepAlive>,
    )
    expect(firstPane).toBeInTheDocument()
    expect((firstPane?.parentElement as HTMLElement).hidden).toBe(true)
    expect(firstPane?.parentElement?.hasAttribute('inert')).toBe(true)

    rerender(
      <ChatRouteKeepAlive activeKey="conversation">
        <main data-testid="chat-pane">chat updated</main>
      </ChatRouteKeepAlive>,
    )
    expect(document.querySelector('[data-testid="chat-pane"]')).toBe(firstPane)
    expect((firstPane?.parentElement as HTMLElement).hidden).toBe(false)
    expect(firstPane).toHaveTextContent('chat updated')
  })
})
