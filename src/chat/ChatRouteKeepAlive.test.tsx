import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { ChatRouteKeepAlive } from './ChatRouteKeepAlive'

describe('ChatRouteKeepAlive', () => {
  it('uses one cached conversation slot for normal Chat and Study, never two composers', () => {
    const { rerender } = render(<ChatRouteKeepAlive activeKey="conversation"><main data-testid="normal">Normal composer</main></ChatRouteKeepAlive>)
    rerender(<ChatRouteKeepAlive activeKey="study"><section data-testid="study">Shared reading composer</section></ChatRouteKeepAlive>)
    expect(document.querySelector('[data-testid="normal"]')).not.toBeInTheDocument()
    const reading = document.querySelector('[data-testid="study"]')
    expect(reading).toBeVisible()
    rerender(<ChatRouteKeepAlive activeKey="settings"><aside>Settings</aside></ChatRouteKeepAlive>)
    expect(reading).toBeInTheDocument()
    expect(reading).not.toBeVisible()
    rerender(<ChatRouteKeepAlive activeKey="conversation"><main data-testid="normal">Normal composer</main></ChatRouteKeepAlive>)
    expect(document.querySelector('[data-testid="study"]')).not.toBeInTheDocument()
    expect(document.querySelector('[data-testid="normal"]')).toBeVisible()
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
