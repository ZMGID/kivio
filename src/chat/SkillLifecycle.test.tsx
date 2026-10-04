import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { open } from '@tauri-apps/plugin-dialog'
import { homeDir } from '@tauri-apps/api/path'
import { api, type Settings, type SkillDetail, type SkillMeta } from '../api/tauri'
import { getSettingsCached, updateSettingsCached } from '../api/settingsCache'
import { listClawHubSkills, resolveClawHubSkillOwner, type ClawHubSkillCard } from '../settings/public/skills'
import { makeChatToolsFixture, makeSettings } from '../settings/tabs/testFixtures'
import { SkillCenter } from './SkillCenter'
import { SkillStoreBrowser } from './SkillStoreBrowser'
import {
  deleteInstalledSkill,
  importSkillFolder,
  installStoreSkill,
  resetSkillLifecycleStoreForTests,
  setSkillEnabled,
} from './skillLifecycle'

vi.mock('../api/tauri', () => ({
  api: {
    chatSkillsList: vi.fn(),
    chatSkillsInstallFromUrl: vi.fn(),
    chatSkillsUninstall: vi.fn(),
    chatSkillsImport: vi.fn(),
    chatSkillsRead: vi.fn(),
    chatSkillsOpenFolder: vi.fn(),
    pluginsListCached: vi.fn(),
    chatPiAgentDir: vi.fn(),
    openExternal: vi.fn(),
  },
  isTauriRuntime: () => false,
}))

vi.mock('../api/settingsCache', () => ({
  getSettingsCached: vi.fn(),
  updateSettingsCached: vi.fn(),
}))

vi.mock('../settings/public/skills', () => ({
  listClawHubSkills: vi.fn(),
  searchClawHubSkills: vi.fn(),
  resolveClawHubSkillOwner: vi.fn(async (card: ClawHubSkillCard) => card),
  buildClawHubDownloadUrl: (slug: string, owner?: string | null) => `https://clawhub.test/${owner ?? 'unknown'}/${slug}`,
  CLAWHUB_SORT_OPTIONS: [
    { value: 'downloads', labelZh: '下载最多', labelEn: 'Most downloaded' },
    { value: 'stars', labelZh: '星标最多', labelEn: 'Most starred' },
    { value: 'installs', labelZh: '安装最多', labelEn: 'Most installed' },
    { value: 'updated', labelZh: '最近更新', labelEn: 'Recently updated' },
    { value: 'newest', labelZh: '最新发布', labelEn: 'Newest' },
  ],
}))

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))
vi.mock('@tauri-apps/api/path', () => ({ homeDir: vi.fn(async () => '/Users/tester') }))

const CARD: ClawHubSkillCard = {
  slug: 'alpha',
  displayName: 'Alpha',
  summary: 'hello',
  latestVersion: '1.0.0',
  downloads: 3,
  stars: 1,
  installsCurrent: 2,
  updatedAt: 1,
  ownerHandle: 'ada',
  webUrl: null,
  downloadUrl: 'https://clawhub.test/alpha.zip',
}

type InstallResult = { success: boolean; skill?: SkillMeta | null; error?: string | null }

const pendingSettles: Array<() => void> = []

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  pendingSettles.push(() => resolve(undefined as T))
  return { promise, resolve, reject }
}

function skill(id: string, extra: Partial<SkillMeta> = {}): SkillMeta {
  return {
    id,
    name: extra.name ?? id,
    description: '',
    source: 'user',
    recommendedTools: [],
    ...extra,
  }
}

function previewDetail(id: string, name: string): SkillDetail {
  return { ...skill(id, { name, description: `${name} detail` }), body: `${name} body` }
}

type SkillReadResult = { success: boolean; skill?: SkillDetail | null; error?: string | null }

function storeCard(name: string): HTMLElement {
  const title = screen.getByText(name)
  const card = title.closest('div.group')
  if (!(card instanceof HTMLElement)) throw new Error(`missing store card ${name}`)
  return card
}

function settings(): Settings {
  return makeSettings({
    chatTools: makeChatToolsFixture({
      enabled: true,
      toolTimeoutMs: 30000,
      approvalPolicy: 'always_confirm',
    }),
  })
}

let installed: SkillMeta[] = []
let cliFound: SkillMeta[] = []
let currentSettings = settings()

function isCliScan(paths?: string[]) {
  return Boolean(paths?.some((path) => path.includes('.claude/skills') || path.includes('.codex/skills') || path.includes('/.pi/')))
}

beforeEach(async () => {
  installed = []
  cliFound = []
  currentSettings = settings()
  resetSkillLifecycleStoreForTests()
  vi.clearAllMocks()
  vi.mocked(getSettingsCached).mockImplementation(async () => currentSettings)
  vi.mocked(updateSettingsCached).mockImplementation(async (mutate) => {
    currentSettings = mutate(currentSettings)
    return currentSettings
  })
  vi.mocked(api.chatSkillsList).mockImplementation(async (paths?: string[]) => ({
    success: true,
    skills: isCliScan(paths) ? cliFound : installed,
  }))
  vi.mocked(api.chatSkillsInstallFromUrl).mockResolvedValue({ success: true, skill: null })
  vi.mocked(api.chatSkillsUninstall).mockResolvedValue(undefined)
  vi.mocked(api.chatSkillsImport).mockResolvedValue({ success: true, skill: null })
  vi.mocked(api.chatPiAgentDir).mockResolvedValue(null)
  vi.mocked(listClawHubSkills).mockResolvedValue({ items: [CARD], nextCursor: null })
  vi.mocked(resolveClawHubSkillOwner).mockImplementation(async (card) => card)
  vi.mocked(open).mockResolvedValue(null)
  vi.mocked(homeDir).mockResolvedValue('/Users/tester')
  await act(async () => {})
})

afterEach(async () => {
  cleanup()
  for (const settle of pendingSettles.splice(0)) settle()
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
  resetSkillLifecycleStoreForTests()
})

describe('skill store install lifetime', () => {
  it('keeps one in-flight install busy across unmount and does not send a second request', async () => {
    const gate = deferred<InstallResult>()
    vi.mocked(api.chatSkillsInstallFromUrl).mockImplementation(() => gate.promise)
    render(<SkillStoreBrowser />)
    await screen.findByRole('button', { name: '安装' })
    void installStoreSkill(CARD, '安装失败')
    void installStoreSkill(CARD, '安装失败')
    await act(async () => {})
    expect(api.chatSkillsInstallFromUrl).toHaveBeenCalledTimes(1)
    expect(api.chatSkillsInstallFromUrl).toHaveBeenCalledWith(CARD.downloadUrl)
    expect(resolveClawHubSkillOwner).toHaveBeenCalledTimes(1)

    cleanup()
    render(<SkillStoreBrowser />)
    expect(await screen.findByRole('button', { name: '安装中…' })).toBeDisabled()
    expect(api.chatSkillsInstallFromUrl).toHaveBeenCalledTimes(1)

    installed = [skill('alpha', { name: 'Alpha Skill' })]
    cleanup()
    await act(async () => {
      gate.resolve({ success: true, skill: installed[0] })
    })
    render(<SkillStoreBrowser />)
    expect(await screen.findByText('已安装')).toBeTruthy()
    expect(api.chatSkillsList).toHaveBeenCalled()
    expect(screen.queryByRole('button', { name: '安装' })).toBeNull()
  })

  it('keeps the failed install error for a later retry', async () => {
    const gate = deferred<InstallResult>()
    vi.mocked(api.chatSkillsInstallFromUrl).mockImplementation(() => gate.promise)
    const view = render(<SkillStoreBrowser />)
    await screen.findByRole('button', { name: '安装' })
    void installStoreSkill(CARD, '安装失败')
    await act(async () => {})
    view.unmount()
    await act(async () => {
      gate.reject(new Error('network down'))
    })
    render(<SkillStoreBrowser />)
    expect(await screen.findByText('network down')).toBeTruthy()
    const retry = screen.getByRole('button', { name: '安装' })
    expect(retry).toBeEnabled()

    const retryGate = deferred<InstallResult>()
    vi.mocked(api.chatSkillsInstallFromUrl).mockImplementation(() => retryGate.promise.then((result) => {
      if (result.success) installed = [skill('alpha', { name: 'Alpha Skill' })]
      return result
    }))
    retry.click()
    await act(async () => {})
    expect(api.chatSkillsInstallFromUrl).toHaveBeenCalledTimes(2)
    await act(async () => {
      retryGate.resolve({ success: true, skill: skill('alpha', { name: 'Alpha Skill' }) })
    })
    expect(await screen.findByText('已安装')).toBeTruthy()
  })

  it('keeps a store install running when the skill center leaves that tab', async () => {
    const gate = deferred<InstallResult>()
    vi.mocked(api.chatSkillsInstallFromUrl).mockImplementation(() => gate.promise.then((result) => {
      if (result.success) installed = [skill('alpha', { name: 'Alpha Skill' })]
      return result
    }))
    const onSkillsChanged = vi.fn()
    const view = render(<SkillCenter onSkillsChanged={onSkillsChanged} />)
    fireEvent.click(screen.getByRole('button', { name: '技能商店' }))
    const install = await screen.findByRole('button', { name: '安装' })
    await act(async () => {
      install.click()
      install.click()
    })
    expect(api.chatSkillsInstallFromUrl).toHaveBeenCalledTimes(1)

    fireEvent.click(screen.getByRole('button', { name: '本地导入' }))
    fireEvent.click(screen.getByRole('button', { name: '技能商店' }))
    expect(await screen.findByRole('button', { name: '安装中…' })).toBeDisabled()
    expect(api.chatSkillsInstallFromUrl).toHaveBeenCalledTimes(1)

    view.unmount()
    const returned = render(<SkillCenter onSkillsChanged={onSkillsChanged} />)
    expect(await screen.findByRole('button', { name: '安装中…' })).toBeDisabled()
    returned.unmount()
    await act(async () => {
      gate.resolve({ success: true, skill: skill('alpha', { name: 'Alpha Skill' }) })
    })
    render(<SkillCenter onSkillsChanged={onSkillsChanged} />)
    expect(onSkillsChanged).toHaveBeenCalled()
    expect(await screen.findByText('已安装', { selector: 'span.chat-motion-pop' })).toBeTruthy()
  })
})

describe('skill import, enable, and delete lifetime', () => {
  it('keeps a URL draft and its install across tab changes, absence, failure, and retry', async () => {
    const gate = deferred<InstallResult>()
    vi.mocked(api.chatSkillsInstallFromUrl).mockImplementation(() => gate.promise)
    const view = render(<SkillCenter />)
    fireEvent.click(screen.getByRole('button', { name: '本地导入' }))
    fireEvent.change(screen.getByPlaceholderText('https://github.com/owner/repo'), {
      target: { value: 'https://github.com/ada/skill' },
    })
    const install = screen.getByRole('button', { name: '安装' })
    await act(async () => {
      install.click()
      install.click()
    })
    expect(api.chatSkillsInstallFromUrl).toHaveBeenCalledTimes(1)
    expect(api.chatSkillsInstallFromUrl).toHaveBeenCalledWith('https://github.com/ada/skill')

    fireEvent.click(screen.getByRole('button', { name: '技能商店' }))
    fireEvent.click(screen.getByRole('button', { name: '本地导入' }))
    expect(screen.getByPlaceholderText('https://github.com/owner/repo')).toHaveValue('https://github.com/ada/skill')
    expect(screen.getByRole('button', { name: '安装中…' })).toBeDisabled()

    view.unmount()
    await act(async () => {
      gate.reject(new Error('url refused'))
    })
    render(<SkillCenter />)
    const input = screen.getByPlaceholderText('https://github.com/owner/repo')
    expect(input).toHaveValue('https://github.com/ada/skill')
    expect(screen.getByText('url refused')).toBeTruthy()

    const retryGate = deferred<InstallResult>()
    vi.mocked(api.chatSkillsInstallFromUrl).mockImplementation(() => retryGate.promise.then((result) => {
      if (result.success) installed = [skill('remote', { name: 'Remote Skill' })]
      return result
    }))
    fireEvent.click(screen.getByRole('button', { name: '安装' }))
    await act(async () => {})
    expect(api.chatSkillsInstallFromUrl).toHaveBeenCalledTimes(2)
    await act(async () => {
      retryGate.resolve({ success: true, skill: installed[0] })
    })
    expect(screen.getByPlaceholderText('https://github.com/owner/repo')).toHaveValue('')
    expect(screen.getByText('已安装', { selector: 'div' })).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /^已安装/ }))
    expect(await screen.findByText('Remote Skill')).toBeTruthy()
  })

  it('keeps a CLI import running across tabs and retries after a failed copy', async () => {
    cliFound = [skill('cli-one', {
      name: 'CLI One',
      source: 'external',
      path: '/Users/tester/.claude/skills/cli-one/SKILL.md',
    })]
    const gate = deferred<InstallResult>()
    vi.mocked(api.chatSkillsImport).mockImplementation(() => gate.promise)
    render(<SkillCenter />)
    fireEvent.click(screen.getByRole('button', { name: '本地导入' }))
    fireEvent.click(screen.getByRole('button', { name: '扫描' }))
    fireEvent.click(await screen.findByRole('checkbox'))
    const start = screen.getByRole('button', { name: '导入选中 (1)' })
    start.click()
    start.click()
    await act(async () => {})
    expect(api.chatSkillsImport).toHaveBeenCalledTimes(1)
    expect(api.chatSkillsImport).toHaveBeenCalledWith('/Users/tester/.claude/skills/cli-one')

    fireEvent.click(screen.getByRole('button', { name: '技能商店' }))
    fireEvent.click(screen.getByRole('button', { name: '本地导入' }))
    expect(screen.getByRole('button', { name: '导入中…' })).toBeDisabled()
    expect(screen.getByRole('checkbox')).toBeChecked()

    cleanup()
    await act(async () => {
      gate.resolve({ success: false, error: 'copy failed' })
    })
    render(<SkillCenter />)
    fireEvent.click(screen.getByRole('button', { name: '本地导入' }))
    expect(screen.getByText('copy failed')).toBeTruthy()
    expect(screen.getByRole('checkbox')).toBeChecked()

    const retryGate = deferred<InstallResult>()
    vi.mocked(api.chatSkillsImport).mockImplementation(() => retryGate.promise.then((result) => {
      if (result.success) installed = [skill('cli-one', { name: 'CLI One' })]
      return result
    }))
    fireEvent.click(screen.getByRole('button', { name: '导入选中 (1)' }))
    await act(async () => {})
    expect(api.chatSkillsImport).toHaveBeenCalledTimes(2)
    await act(async () => {
      retryGate.resolve({ success: true, skill: installed[0] })
    })
    expect(screen.getByText('已导入 1 个技能到「已安装」。')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /^已安装/ }))
    expect(await screen.findByText('CLI One')).toBeTruthy()
  })

  it('keeps an enable draft across absence and retries the failed save', async () => {
    installed = [skill('alpha', { name: 'Alpha Skill' })]
    const gate = deferred<boolean>()
    vi.mocked(updateSettingsCached).mockImplementation((mutate) => gate.promise.then((ok) => {
      if (!ok) throw new Error('save failed')
      currentSettings = mutate(currentSettings)
      return currentSettings
    }))
    render(<SkillCenter />)
    const toggle = await screen.findByRole('switch', { name: '启用 Alpha Skill' })
    expect(toggle).toHaveAttribute('aria-checked', 'true')
    void setSkillEnabled('alpha', false)
    void setSkillEnabled('alpha', false)
    await act(async () => {})
    expect(updateSettingsCached).toHaveBeenCalledTimes(1)
    expect(screen.getByRole('switch', { name: '启用 Alpha Skill' })).toHaveAttribute('aria-checked', 'false')
    expect(screen.getByRole('switch', { name: '启用 Alpha Skill' })).toBeDisabled()

    cleanup()
    render(<SkillCenter />)
    expect(await screen.findByRole('switch', { name: '启用 Alpha Skill' })).toHaveAttribute('aria-checked', 'false')
    expect(updateSettingsCached).toHaveBeenCalledTimes(1)

    cleanup()
    await act(async () => {
      gate.resolve(false)
    })
    render(<SkillCenter />)
    expect(await screen.findByText('save failed')).toBeTruthy()
    expect(screen.getByRole('switch', { name: '启用 Alpha Skill' })).toHaveAttribute('aria-checked', 'false')
    expect(screen.getByRole('switch', { name: '启用 Alpha Skill' })).toBeEnabled()

    const retryGate = deferred<boolean>()
    vi.mocked(updateSettingsCached).mockImplementation((mutate) => retryGate.promise.then((ok) => {
      if (!ok) throw new Error('save failed')
      currentSettings = mutate(currentSettings)
      return currentSettings
    }))
    fireEvent.click(screen.getByRole('button', { name: '重试' }))
    await act(async () => {})
    expect(updateSettingsCached).toHaveBeenCalledTimes(2)
    await act(async () => {
      retryGate.resolve(true)
    })
    expect(screen.queryByText('save failed')).toBeNull()
    expect(screen.getByRole('switch', { name: '启用 Alpha Skill' })).toHaveAttribute('aria-checked', 'false')
    expect(currentSettings.chatTools.disabledSkillIds).toEqual(['alpha'])
  })

  it('deletes once, ignores a cancelled confirm, and refreshes inventory after the uninstall finishes away', async () => {
    installed = [skill('alpha', { name: 'Alpha Skill' })]
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false)
    const target = installed[0]
    void deleteInstalledSkill(target, { confirm: '确定删除技能「Alpha Skill」？', confirmLabel: '删除' })
    void deleteInstalledSkill(target, { confirm: '确定删除技能「Alpha Skill」？', confirmLabel: '删除' })
    await act(async () => {})
    expect(confirm).toHaveBeenCalledTimes(1)
    expect(api.chatSkillsUninstall).not.toHaveBeenCalled()

    confirm.mockReturnValue(true)
    const gate = deferred<void>()
    vi.mocked(api.chatSkillsUninstall).mockImplementation(() => gate.promise)
    render(<SkillCenter />)
    fireEvent.click(await screen.findByRole('button', { name: '删除 Alpha Skill' }))
    await act(async () => {})
    expect(api.chatSkillsUninstall).toHaveBeenCalledTimes(1)
    expect(api.chatSkillsUninstall).toHaveBeenCalledWith('alpha')
    expect(screen.getByRole('button', { name: '删除 Alpha Skill' })).toBeDisabled()

    cleanup()
    render(<SkillCenter />)
    expect(await screen.findByRole('button', { name: '删除 Alpha Skill' })).toBeDisabled()
    expect(api.chatSkillsUninstall).toHaveBeenCalledTimes(1)

    cleanup()
    installed = []
    await act(async () => {
      gate.resolve()
    })
    render(<SkillCenter />)
    expect(await screen.findByText('当前没有个人技能。')).toBeTruthy()
    expect(screen.queryByText('Alpha Skill')).toBeNull()
  })

  it('treats a cancelled folder picker as cancel and still accepts a later import', async () => {
    const picker = deferred<string | null>()
    vi.mocked(open).mockImplementation(() => picker.promise as Promise<null>)
    void importSkillFolder('导入失败')
    void importSkillFolder('导入失败')
    await act(async () => {})
    expect(open).toHaveBeenCalledTimes(1)
    await act(async () => {
      picker.resolve(null)
    })
    expect(api.chatSkillsImport).not.toHaveBeenCalled()

    const again = deferred<string | null>()
    vi.mocked(open).mockImplementation(() => again.promise as Promise<null>)
    void importSkillFolder('导入失败')
    await act(async () => {})
    expect(open).toHaveBeenCalledTimes(2)
    installed = [skill('folder', { name: 'Folder Skill' })]
    await act(async () => {
      again.resolve('/tmp/folder-skill')
    })
    expect(api.chatSkillsImport).toHaveBeenCalledTimes(1)
    expect(api.chatSkillsImport).toHaveBeenCalledWith('/tmp/folder-skill')
    render(<SkillCenter />)
    expect(await screen.findByText('Folder Skill')).toBeTruthy()
  })
})

describe('store install marker follows the inventory skill', () => {
  const BETA: ClawHubSkillCard = {
    ...CARD,
    slug: 'beta',
    displayName: 'Beta',
    downloadUrl: 'https://clawhub.test/beta.zip',
  }

  it('offers reinstall after uninstall and keeps the badge when uninstall fails', async () => {
    vi.mocked(listClawHubSkills).mockResolvedValue({ items: [CARD, BETA], nextCursor: null })
    vi.mocked(api.chatSkillsInstallFromUrl).mockImplementation(async (url: string) => {
      const next = url.includes('/beta.zip')
        ? skill('beta-pack', { name: 'Beta Pack' })
        : skill('alpha-pack', { name: 'Alpha Pack' })
      installed = [...installed.filter((item) => item.id !== next.id), next]
      return { success: true, skill: next }
    })
    let rejectUninstall = true
    vi.mocked(api.chatSkillsUninstall).mockImplementation(async (id: string) => {
      if (rejectUninstall) throw new Error('uninstall failed')
      installed = installed.filter((item) => item.id !== id)
    })
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true)
    try {
      render(<SkillCenter />)
      fireEvent.click(screen.getByRole('button', { name: '技能商店' }))
      expect(await screen.findByText('Alpha')).toBeTruthy()
      fireEvent.click(within(storeCard('Alpha')).getByRole('button', { name: '安装' }))
      expect(await within(storeCard('Alpha')).findByText('已安装')).toBeTruthy()
      fireEvent.click(within(storeCard('Beta')).getByRole('button', { name: '安装' }))
      expect(await within(storeCard('Beta')).findByText('已安装')).toBeTruthy()

      fireEvent.click(screen.getByRole('button', { name: /^已安装/ }))
      fireEvent.click(await screen.findByRole('button', { name: '删除 Alpha Pack' }))
      expect(await screen.findByText('uninstall failed')).toBeTruthy()
      fireEvent.click(screen.getByRole('button', { name: '技能商店' }))
      expect(await screen.findByText('Alpha')).toBeTruthy()
      expect(within(storeCard('Alpha')).getByText('已安装')).toBeTruthy()
      expect(within(storeCard('Beta')).getByText('已安装')).toBeTruthy()
      expect(within(storeCard('Alpha')).queryByRole('button', { name: '安装' })).toBeNull()

      rejectUninstall = false
      fireEvent.click(screen.getByRole('button', { name: /^已安装/ }))
      fireEvent.click(await screen.findByRole('button', { name: '删除 Alpha Pack' }))
      await waitFor(() => {
        expect(screen.queryByText('Alpha Pack')).toBeNull()
      })
      expect(screen.getByText('Beta Pack')).toBeTruthy()
      fireEvent.click(screen.getByRole('button', { name: '技能商店' }))
      expect(await screen.findByText('Alpha')).toBeTruthy()
      expect(within(storeCard('Alpha')).getByRole('button', { name: '安装' })).toBeEnabled()
      expect(within(storeCard('Alpha')).queryByText('已安装')).toBeNull()
      expect(within(storeCard('Beta')).getByText('已安装')).toBeTruthy()
    } finally {
      confirm.mockRestore()
    }
  })
})

describe('skill preview ownership', () => {
  it('ignores a preview failure after unmount and after newer feedback', async () => {
    installed = [skill('alpha', { name: 'Alpha Pack' })]
    const first = deferred<SkillReadResult>()
    vi.mocked(api.chatSkillsRead).mockImplementation(() => first.promise)
    const view = render(<SkillCenter />)
    fireEvent.click(await screen.findByText('Alpha Pack'))
    view.unmount()
    await act(async () => {
      first.reject(new Error('stale read'))
    })
    render(<SkillCenter />)
    expect(await screen.findByText('Alpha Pack')).toBeTruthy()
    expect(screen.queryByText('stale read')).toBeNull()

    const second = deferred<SkillReadResult>()
    vi.mocked(api.chatSkillsRead).mockImplementation(() => second.promise)
    fireEvent.click(screen.getByText('Alpha Pack'))
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true)
    vi.mocked(api.chatSkillsUninstall).mockRejectedValue(new Error('uninstall failed'))
    try {
      fireEvent.click(screen.getByRole('button', { name: '删除 Alpha Pack' }))
      expect(await screen.findByText('uninstall failed')).toBeTruthy()
      await act(async () => {
        second.reject(new Error('later read'))
      })
      expect(screen.getByText('uninstall failed')).toBeTruthy()
      expect(screen.queryByText('later read')).toBeNull()
      expect(screen.queryByText('stale read')).toBeNull()
    } finally {
      confirm.mockRestore()
    }
  })

  it('drops a preview that finishes after leaving and returning to the installed tab', async () => {
    installed = [skill('alpha', { name: 'Alpha Pack' })]
    const succeeded = deferred<SkillReadResult>()
    const failed = deferred<SkillReadResult>()
    const reads = [succeeded, failed]
    vi.mocked(api.chatSkillsRead).mockImplementation(() => {
      const next = reads.shift()
      if (!next) throw new Error('unexpected preview read')
      return next.promise
    })
    render(<SkillCenter />)
    fireEvent.click(await screen.findByText('Alpha Pack'))
    fireEvent.click(screen.getByRole('button', { name: '技能商店' }))
    fireEvent.click(screen.getByRole('button', { name: /^已安装/ }))
    await act(async () => {
      succeeded.resolve({ success: true, skill: previewDetail('alpha', 'Alpha Pack') })
    })
    expect(screen.queryByRole('dialog')).toBeNull()

    fireEvent.click(screen.getByText('Alpha Pack'))
    fireEvent.click(screen.getByRole('button', { name: '本地导入' }))
    fireEvent.click(screen.getByRole('button', { name: /^已安装/ }))
    await act(async () => {
      failed.reject(new Error('stale read'))
    })
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(screen.queryByText('stale read')).toBeNull()
  })

  it('keeps the newest preview when an older read finishes later', async () => {
    installed = [
      skill('alpha', { name: 'Alpha Pack' }),
      skill('beta', { name: 'Beta Pack' }),
    ]
    const pending = new Map<string, ReturnType<typeof deferred<SkillReadResult>>>()
    vi.mocked(api.chatSkillsRead).mockImplementation((id: string) => {
      const gate = deferred<SkillReadResult>()
      pending.set(id, gate)
      return gate.promise
    })
    render(<SkillCenter />)
    fireEvent.click(await screen.findByText('Alpha Pack'))
    fireEvent.click(screen.getByText('Beta Pack'))
    const alpha = pending.get('alpha')
    const beta = pending.get('beta')
    if (!alpha || !beta) throw new Error('preview reads did not start')
    await act(async () => {
      beta.resolve({ success: true, skill: previewDetail('beta', 'Beta Pack') })
    })
    expect(await screen.findByRole('dialog', { name: 'Beta Pack' })).toBeTruthy()
    await act(async () => {
      alpha.resolve({ success: true, skill: previewDetail('alpha', 'Alpha Pack') })
    })
    expect(screen.getByRole('dialog', { name: 'Beta Pack' })).toBeTruthy()
    expect(screen.queryByRole('dialog', { name: 'Alpha Pack' })).toBeNull()
  })
})
