import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { PullRequestsPanel } from './PullRequestsPanel'
import { resetPullRequestCache, writeCache } from './pullRequestCache'
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
const openCalls = () => mocks.pullRequests.mock.calls.filter((call) => call[1] === 'open').length

beforeEach(() => {
  vi.resetAllMocks()
  resetPullRequestCache()
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

it('keeps controls during the first load and switches states instantly after prefetch', async () => {
  const open = deferred<PullRequestList>()
  mocks.pullRequests.mockImplementation((_dir: string, state: string) =>
    state === 'open' ? open.promise : Promise.resolve(list({ totalCount: 1, items: [pr({ number: 5, title: `${state} work`, state: state === 'all' ? 'open' : state as PullRequestItem['state'] })] })))
  render(<PullRequestsPanel workdir={nextWorkdir()} active lang="en" />)
  await act(async () => {})

  // 首页未到：搜索框与状态分段已经在页面上，只有列表区域是骨架。
  expect(screen.getByRole('textbox', { name: /Search pull requests/ })).toBeInTheDocument()
  expect(screen.getByRole('radio', { name: /Merged/ })).toBeInTheDocument()
  expect(mocks.pullRequests).toHaveBeenCalledTimes(1)

  open.resolve(list({ totalCount: 1, items: [pr({ number: 9, title: 'Open work' })] }))
  expect(await screen.findByText('Open work')).toBeInTheDocument()
  // 首页到达后在后台预取其余三个状态。
  await waitFor(() => expect(mocks.pullRequests).toHaveBeenCalledTimes(4))
  expect(mocks.pullRequests.mock.calls.map((call) => call[1]).sort()).toEqual(['all', 'closed', 'merged', 'open'])

  fireEvent.click(screen.getByRole('radio', { name: /Merged/ }))
  expect(screen.getByText('merged work')).toBeInTheDocument()
  fireEvent.click(screen.getByRole('radio', { name: /Open/ }))
  expect(screen.getByText('Open work')).toBeInTheDocument()
  expect(mocks.pullRequests).toHaveBeenCalledTimes(4)
})

it('shows a dimmed placeholder from other cached states while a state is still loading', async () => {
  const merged = deferred<PullRequestList>()
  mocks.pullRequests.mockImplementation((_dir: string, state: string) => {
    if (state === 'merged') return merged.promise
    if (state === 'all') return Promise.resolve(list({ totalCount: 2, items: [pr({ number: 9, title: 'Open work' }), pr({ number: 5, title: 'Merged work', state: 'merged' })] }))
    return Promise.resolve(list({ totalCount: 1, items: [pr({ number: 9, title: 'Open work' })] }))
  })
  render(<PullRequestsPanel workdir={nextWorkdir()} active lang="en" />)
  expect(await screen.findByText('Open work')).toBeInTheDocument()
  await waitFor(() => expect(mocks.pullRequests).toHaveBeenCalledTimes(4))

  fireEvent.click(screen.getByRole('radio', { name: /Merged/ }))
  expect(screen.getByText('Merged work')).toBeInTheDocument()
  expect(screen.queryByText('Open work')).toBeNull()
  expect(screen.getByText('Merged work').closest('[aria-busy]')).not.toBeNull()

  // 真实结果到达后替换占位；切换过程中没有重复请求（复用在途的预取）。
  merged.resolve(list({ totalCount: 1, items: [pr({ number: 5, title: 'Merged work (fresh)', state: 'merged' })] }))
  expect(await screen.findByText('Merged work (fresh)')).toBeInTheDocument()
  expect(screen.getByText('Merged work (fresh)').closest('[aria-busy]')).toBeNull()
  expect(mocks.pullRequests).toHaveBeenCalledTimes(4)
})

it('paints the persisted list immediately after a restart and refreshes stale data in the background', async () => {
  const workdir = nextWorkdir()
  writeCache(`${workdir}\nopen`, list({ totalCount: 1, items: [pr({ title: 'From last session' })] }), Date.now() - 5 * 60_000)
  resetPullRequestCache({ keepStorage: true })
  const fresh = deferred<PullRequestList>()
  mocks.pullRequests.mockImplementation((_dir: string, state: string) => (state === 'open' ? fresh.promise : Promise.resolve(list())))

  render(<PullRequestsPanel workdir={workdir} active lang="en" />)
  expect(screen.getByText('From last session')).toBeInTheDocument()
  expect(mocks.pullRequests).toHaveBeenCalledWith(workdir, 'open')

  fresh.resolve(list({ totalCount: 1, items: [pr({ title: 'Fresh from GitHub' })] }))
  expect(await screen.findByText('Fresh from GitHub')).toBeInTheDocument()
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
  expect(mocks.pullRequests).toHaveBeenCalledWith(workdir, 'open')
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
  await waitFor(() => expect(openCalls()).toBe(2))
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
  await waitFor(() => expect(openCalls()).toBe(2))
})

it('hides the branch card on the default branch without a pull request', async () => {
  mocks.pullRequests.mockResolvedValue(list({ items: [pr({ title: 'Someone else' })], totalCount: 1 }))
  render(<PullRequestsPanel workdir={nextWorkdir()} active lang="en" />)
  expect(await screen.findByText('Someone else')).toBeInTheDocument()
  expect(screen.queryByText('Current branch')).toBeNull()
})
