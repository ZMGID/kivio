import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { PluginContents } from './PluginContents'
import { marketplaceApi } from '../../api/market'
import { packageApi, type PluginDetails } from '../../api/pluginPackages'

vi.mock('../../api/market', () => ({ marketplaceApi: { describe: vi.fn() } }))
vi.mock('../../api/pluginPackages', () => ({ packageApi: { describe: vi.fn() } }))
vi.mock('../../api/tauri', () => ({ api: { openExternal: vi.fn() } }))
const details: PluginDetails = {
  author: 'Adobe', version: '2.0', homepage: 'https://example.com', license: 'MIT', diagnostics: [],
  groups: [{ kind: 'mcp', items: [{ name: 'Adobe for creativity', description: '' }] }, { kind: 'skills', items: [{ name: 'adobe-edit', description: 'Edit photos consistently' }] }],
}
beforeEach(() => vi.resetAllMocks())

it('loads marketplace contents before installation, then retries a failed read', async () => {
  vi.mocked(marketplaceApi.describe).mockRejectedValueOnce('Network unavailable').mockResolvedValueOnce(details)
  render(<PluginContents marketplaceId="official" plugin="adobe" version={null} information={<><dt>来源</dt><dd>Official</dd></>} />)
  expect(screen.getByText('正在读取插件内容…')).toBeInTheDocument()
  await screen.findByText(/Network unavailable/)
  fireEvent.click(screen.getByRole('button', { name: '重试' }))
  await screen.findByText('adobe-edit')
  expect(screen.getByText('Edit photos consistently')).toHaveAttribute('title', 'Edit photos consistently')
  expect(screen.getByText('Adobe for creativity')).toBeInTheDocument()
  expect(screen.getByText('Adobe')).toBeInTheDocument()
  expect(screen.getByText('2.0')).toBeInTheDocument()
  expect(marketplaceApi.describe).toHaveBeenLastCalledWith('official', 'adobe')
  expect(packageApi.describe).not.toHaveBeenCalled()
})

it('isolates late remote results and reads installed packages locally', async () => {
  let finish!: (value: PluginDetails) => void
  vi.mocked(marketplaceApi.describe).mockReturnValue(new Promise(resolve => { finish = resolve }))
  vi.mocked(packageApi.describe).mockResolvedValue({ ...details, author: 'Local author' })
  const { rerender } = render(<PluginContents key="remote" marketplaceId="official" plugin="adobe" version={null} information={null} />)
  rerender(<PluginContents key="local" packageId="installed-id" version={null} information={null} />)
  await screen.findByText('Local author')
  await act(async () => finish(details))
  expect(screen.queryByText('Adobe')).not.toBeInTheDocument()
  expect(packageApi.describe).toHaveBeenCalledWith('installed-id')
})
