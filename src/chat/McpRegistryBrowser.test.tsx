import { screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { render } from '../test/renderWithNotices'
import { i18n } from '../components/i18n'
import { searchMcpRegistry } from '../settings/public/mcpRegistry'
import { McpRegistryBrowser } from './McpRegistryBrowser'

vi.mock('../settings/public/mcpRegistry', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../settings/public/mcpRegistry')>()),
  searchMcpRegistry: vi.fn(),
}))

describe('McpRegistryBrowser load failure', () => {
  it('keeps the failed load in place with a retry instead of an empty result', async () => {
    vi.mocked(searchMcpRegistry)
      .mockRejectedValueOnce(new Error('registry offline'))
      .mockResolvedValueOnce({ source: 'official', items: [], nextCursor: undefined })
    const user = userEvent.setup()
    render(<McpRegistryBrowser existingServers={[]} onInstall={vi.fn()} />)

    expect(await screen.findByText('registry offline')).toBeInTheDocument()
    expect(screen.queryByText(i18n.zh.chatMcpNoMatch)).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: i18n.zh.chatRetry }))
    expect(await screen.findByText(i18n.zh.chatMcpNoMatch)).toBeInTheDocument()
    expect(screen.queryByText('registry offline')).not.toBeInTheDocument()
    expect(searchMcpRegistry).toHaveBeenCalledTimes(2)
  })
})
