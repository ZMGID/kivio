import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { PullRequestsPanel } from './PullRequestsPanel'
import type { PullRequestDetail, PullRequestItem, PullRequestList } from '../../api/dockContracts'

const mocks = vi.hoisted(() => ({
  pullRequests: vi.fn(),
  pullRequestDetail: vi.fn(),
  pullRequestCheckout: vi.fn(),
  pullRequestCreate: vi.fn(),
  gitLog: vi.fn(),
  gitBranches: vi.fn(),
  openExternal: vi.fn(),
}))
vi.mock('./api', () => ({ dockApi: mocks }))
vi.mock('../../api/tauri', () => ({ api: { openExternal: mocks.openExternal } }))
vi.mock('../ChatMarkdown', () => ({ ChatMarkdown: ({ content }: { content: string }) => <div>{content}</div> }))

function pr(overrides: Partial<PullRequestItem> = {}): PullRequestItem {
  return {
    number: 1, title: 'Untitled', url: 'https://github.com/o/r/pull/1', state: 'open',
    createdAt: '2026-11-01T00:00:00Z', updatedAt: '2026-11-02T00:00:00Z',
    additions: 10, deletions: 2, changedFiles: 3, headRef: 'feat/x', baseRef: 'main',
    crossRepository: false, author: 'other', authorAvatar: '', labels: [], comments: 0,
    checks: 'none', reviewDecision: '', conflicting: false, reviewRequested: false,
    ...overrides,
  }
}

function list(overrides: Partial<PullRequestList> = {}): PullRequestList {
  return {
    status: 'ready', hint: '', viewer: 'me', repository: 'o/r', repositoryUrl: 'https://github.com/o/r',
    defaultBranch: 'main', currentBranch: 'main', current: null, totalCount: 0, items: [],
    endCursor: null, hasMore: false,
    ...overrides,
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((yes) => { resolve = yes })
  return { promise, resolve }
}

const detail: PullRequestDetail = {
  number: 70,
  body: 'Adds FreeRoute',
  checks: [
    { name: 'quality', workflow: 'CI', status: 'failure', url: 'https://ci/1' },
    { name: 'windows', workflow: 'CI', status: 'success', url: '' },
  ],
  reviews: [{ author: 'rev', state: 'approved' }],
  files: [{ path: 'src/a.ts', additions: 3, deletions: 1 }],
}

let dir = 0
const nextWorkdir = () => `/repo-${++dir}`

beforeEach(() => {
  vi.resetAllMocks()
  mocks.openExternal.mockResolvedValue(undefined)
  mocks.gitLog.mockResolvedValue({ commits: [{ subject: 'feat: last commit' }], hasMore: false })
  mocks.gitBranches.mockResolvedValue({ branches: [], remotes: ['origin'], remoteBranches: [{ name: 'origin/develop', remote: 'origin' }], defaultBranch: 'main' })
})
afterEach(cleanup)

it('groups pull requests and narrows them with search qualifiers', async () => {
  mocks.pullRequests.mockResolvedValue(list({
    totalCount: 3,
    items: [
      pr({ number: 70, title: 'feat: FreeRoute', author: 'me', labels: [{ name: 'feature', color: '0e8a16' }] }),
      pr({ number: 61, title: 'feat: remote model database', reviewRequested: true }),
      pr({ number: 8, title: 'bootstrap Linux AppImage' }),
    ],
  }))
  render(<PullRequestsPanel workdir={nextWorkdir()} active lang="en" />)

  expect(await screen.findByText('feat: FreeRoute')).toBeInTheDocument()
  expect(mocks.pullRequests).toHaveBeenCalledWith(expect.any(String), 'open')
  const groups = screen.getAllByRole('region').map((region) => region.getAttribute('aria-label'))
  expect(groups).toEqual(['Review requested', 'Authored', 'Others'])
  expect(within(screen.getByRole('radiogroup')).getByRole('radio', { name: /Open/ })).toHaveTextContent('3')

  fireEvent.change(screen.getByRole('textbox', { name: /Search pull requests/ }), { target: { value: 'label:feature' } })
  expect(screen.getByText('feat: FreeRoute')).toBeInTheDocument()
  expect(screen.queryByText('bootstrap Linux AppImage')).toBeNull()

  fireEvent.change(screen.getByRole('textbox', { name: /Search pull requests/ }), { target: { value: 'nothing-here' } })
  expect(screen.getByText('No pull requests match')).toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }))
  expect(screen.getByText('bootstrap Linux AppImage')).toBeInTheDocument()
})

it('switches state filters without letting a late result overwrite the new view', async () => {
  const open = deferred<PullRequestList>()
  mocks.pullRequests.mockImplementation((_dir: string, state: string) =>
    state === 'open' ? open.promise : Promise.resolve(list({ totalCount: 1, items: [pr({ number: 5, title: 'Merged work', state: 'merged' })] })))
  render(<PullRequestsPanel workdir={nextWorkdir()} active lang="en" />)
  await act(async () => {})

  // 首屏骨架期间状态分段不可见，先让 open 完成再切换。
  open.resolve(list({ totalCount: 1, items: [pr({ number: 9, title: 'Open work' })] }))
  expect(await screen.findByText('Open work')).toBeInTheDocument()

  const late = deferred<PullRequestList>()
  mocks.pullRequests.mockImplementation((_dir: string, state: string) =>
    state === 'open' ? late.promise : Promise.resolve(list({ totalCount: 1, items: [pr({ number: 5, title: 'Merged work', state: 'merged' })] })))
  fireEvent.click(screen.getByRole('radio', { name: /Merged/ }))
  expect(await screen.findByText('Merged work')).toBeInTheDocument()
  expect(mocks.pullRequests).toHaveBeenLastCalledWith(expect.any(String), 'merged')

  // 回到 open：缓存仍新鲜，不再请求；迟到的旧结果也不会出现。
  fireEvent.click(screen.getByRole('radio', { name: /Open/ }))
  expect(screen.getByText('Open work')).toBeInTheDocument()
  expect(mocks.pullRequests).toHaveBeenCalledTimes(2)
  late.resolve(list({ items: [pr({ title: 'Stale' })] }))
  await act(async () => {})
  expect(screen.queryByText('Stale')).toBeNull()
})

it('shows GitHub CLI guidance and retries', async () => {
  mocks.pullRequests.mockResolvedValue(list({ status: 'gh_missing' }))
  render(<PullRequestsPanel workdir={nextWorkdir()} active lang="en" />)
  expect(await screen.findByText('GitHub CLI required')).toBeInTheDocument()

  fireEvent.click(screen.getByRole('button', { name: /Install guide/ }))
  expect(mocks.openExternal).toHaveBeenCalledWith('https://cli.github.com/')

  mocks.pullRequests.mockResolvedValue(list({ totalCount: 1, items: [pr({ title: 'Now visible' })] }))
  fireEvent.click(screen.getByRole('button', { name: /Retry/ }))
  expect(await screen.findByText('Now visible')).toBeInTheDocument()
})

it('does not request while hidden', async () => {
  mocks.pullRequests.mockResolvedValue(list())
  const workdir = nextWorkdir()
  const view = render(<PullRequestsPanel workdir={workdir} active={false} lang="en" />)
  await act(async () => {})
  expect(mocks.pullRequests).not.toHaveBeenCalled()
  view.rerender(<PullRequestsPanel workdir={workdir} active lang="en" />)
  await waitFor(() => expect(mocks.pullRequests).toHaveBeenCalledTimes(1))
})

it('expands a pull request, loads its detail once, and checks it out', async () => {
  mocks.pullRequests.mockResolvedValue(list({
    totalCount: 1,
    items: [pr({ number: 70, title: 'feat: FreeRoute', headRef: 'feat/freeroute', checks: 'failure', conflicting: true })],
  }))
  mocks.pullRequestDetail.mockResolvedValue(detail)
  mocks.pullRequestCheckout.mockResolvedValue({ ok: true, message: '', stdout: '', stderr: '', state: {} })
  render(<PullRequestsPanel workdir={nextWorkdir()} active lang="en" />)

  const row = await screen.findByRole('button', { name: /feat: FreeRoute/ })
  expect(row).toHaveAttribute('aria-expanded', 'false')
  fireEvent.click(row)
  expect(row).toHaveAttribute('aria-expanded', 'true')
  expect(await screen.findByText('Adds FreeRoute')).toBeInTheDocument()
  expect(screen.getByText('1/2 passed')).toBeInTheDocument()
  expect(screen.getByText('Has merge conflicts')).toBeInTheDocument()

  fireEvent.click(screen.getByRole('button', { name: /quality/ }))
  expect(mocks.openExternal).toHaveBeenCalledWith('https://ci/1')

  fireEvent.click(row)
  fireEvent.click(row)
  expect(mocks.pullRequestDetail).toHaveBeenCalledTimes(1)

  fireEvent.click(screen.getByRole('button', { name: /Check out/ }))
  await waitFor(() => expect(mocks.pullRequestCheckout).toHaveBeenCalledWith(expect.any(String), 70))
  await waitFor(() => expect(mocks.pullRequests).toHaveBeenCalledTimes(2))
})

it('creates a pull request for the current branch', async () => {
  mocks.pullRequests.mockResolvedValue(list({ currentBranch: 'feat/new' }))
  mocks.pullRequestCreate.mockResolvedValue({ url: 'https://github.com/o/r/pull/2' })
  const workdir = nextWorkdir()
  render(<PullRequestsPanel workdir={workdir} active lang="en" />)

  expect(await screen.findByText('No pull request for this branch yet')).toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: /Create pull request/ }))
  const title = screen.getByRole('textbox', { name: 'Title' })
  await waitFor(() => expect(title).toHaveValue('feat: last commit'))

  fireEvent.change(title, { target: { value: 'feat: new panel' } })
  fireEvent.click(screen.getByRole('switch', { name: 'Create as draft' }))
  fireEvent.click(screen.getByRole('button', { name: 'Create' }))
  await waitFor(() => expect(mocks.pullRequestCreate).toHaveBeenCalledWith(workdir, {
    title: 'feat: new panel', body: '', base: 'main', draft: true,
  }))
  await waitFor(() => expect(mocks.pullRequests).toHaveBeenCalledTimes(2))
})

it('hides the branch card on the default branch without a pull request', async () => {
  mocks.pullRequests.mockResolvedValue(list({ items: [pr({ title: 'Someone else' })], totalCount: 1 }))
  render(<PullRequestsPanel workdir={nextWorkdir()} active lang="en" />)
  expect(await screen.findByText('Someone else')).toBeInTheDocument()
  expect(screen.queryByText('Current branch')).toBeNull()
})
