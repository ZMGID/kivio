import { fireEvent, render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { McpServerIcon } from './McpServerIcon'

const light = 'data:image/png;base64,bGlnaHQ='
const dark = 'data:image/png;base64,ZGFyaw=='

describe('MCP server icons', () => {
  it('uses the advertised theme variants without a remote browser request', () => {
    const { container } = render(<McpServerIcon icons={[{ src: light, theme: 'light' }, { src: dark, theme: 'dark' }]} />)
    const images = container.querySelectorAll('img')
    expect([...images].map(image => image.src)).toEqual([light, dark])
    expect(images[0].parentElement).toHaveClass('dark:hidden')
    expect(images[1].parentElement).toHaveClass('dark:flex')
    expect(images[0]).toHaveAttribute('alt', '')
  })

  it('falls back on missing, unsafe, and failed icons, and recovers for a new source', () => {
    const view = render(<McpServerIcon />)
    expect(view.container.querySelector('img')).toBeNull()
    view.rerender(<McpServerIcon icons={[{ src: 'https://third-party.example/icon.png' }]} />)
    expect(view.container.querySelector('img')).toBeNull()
    view.rerender(<McpServerIcon icons={[{ src: light }]} />)
    for (const image of view.container.querySelectorAll('img')) fireEvent.error(image)
    expect(view.container.querySelector('img')).toBeNull()
    view.rerender(<McpServerIcon icons={[{ src: dark }]} />)
    expect(view.container.querySelector('img')).toHaveAttribute('src', dark)
  })
})
