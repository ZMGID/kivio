import { render, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { dockApi } from './dock/api'
import { ProjectIcon } from './ProjectIcon'

describe('ProjectIcon', () => {
  it('falls back to a colored initial when the project has no logo', async () => {
    const read = vi.spyOn(dockApi, 'projectIcon').mockResolvedValue(null)
    const { container } = render(<ProjectIcon workdir="/no-logo" name="kivio mobile" />)
    await waitFor(() => expect(read).toHaveBeenCalled())
    const badge = container.querySelector('span')!
    expect(badge).toHaveTextContent('K')
    expect(badge.style.backgroundColor).not.toBe('')
    expect(container.querySelector('img')).not.toBeInTheDocument()
  })

  it('uses the project color when set and keeps the same color for the same name', () => {
    const a = render(<ProjectIcon name="测试" color="#ff0000" />)
    expect(a.container.querySelector('span')).toHaveTextContent('测')
    expect(a.container.querySelector('span')!.style.backgroundColor).toBe('rgb(255, 0, 0)')
    const b = render(<ProjectIcon name="dsivio" />)
    const c = render(<ProjectIcon name="dsivio" />)
    expect(b.container.querySelector('span')!.style.backgroundColor)
      .toBe(c.container.querySelector('span')!.style.backgroundColor)
  })
})
