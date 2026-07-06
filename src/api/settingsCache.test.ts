import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Settings } from './tauri'

const getSettingsMock = vi.fn()
const saveSettingsMock = vi.fn()
const importSettingsMock = vi.fn()
const setFavoriteModelsMock = vi.fn()

vi.mock('./tauri', () => ({
  api: {
    getSettings: (...args: unknown[]) => getSettingsMock(...args),
    saveSettings: (...args: unknown[]) => saveSettingsMock(...args),
    importSettings: (...args: unknown[]) => importSettingsMock(...args),
    setFavoriteModels: (...args: unknown[]) => setFavoriteModelsMock(...args),
  },
}))

import {
  __resetSettingsCacheForTest,
  getSettingsCached,
  importSettingsCached,
  peekSettings,
  refreshSettings,
  saveSettingsCached,
  setFavoriteModelsCached,
} from './settingsCache'

const settingsA = { theme: 'dark', providers: [], favoriteModels: [] } as unknown as Settings
const settingsB = { theme: 'light', providers: [], favoriteModels: [] } as unknown as Settings

beforeEach(() => {
  __resetSettingsCacheForTest()
  getSettingsMock.mockReset()
  saveSettingsMock.mockReset()
  importSettingsMock.mockReset()
  setFavoriteModelsMock.mockReset()
})

describe('settingsCache', () => {
  it('并发首读只发一次 invoke，之后命中缓存不再发', async () => {
    getSettingsMock.mockResolvedValue(settingsA)
    const [first, second] = await Promise.all([getSettingsCached(), getSettingsCached()])
    expect(first).toBe(settingsA)
    expect(second).toBe(settingsA)
    expect(getSettingsMock).toHaveBeenCalledTimes(1)

    await getSettingsCached()
    expect(getSettingsMock).toHaveBeenCalledTimes(1)
  })

  it('首读失败不写缓存，下次调用重试', async () => {
    getSettingsMock.mockRejectedValueOnce(new Error('ipc down'))
    await expect(getSettingsCached()).rejects.toThrow('ipc down')
    expect(peekSettings()).toBeNull()

    getSettingsMock.mockResolvedValue(settingsA)
    await expect(getSettingsCached()).resolves.toBe(settingsA)
    expect(getSettingsMock).toHaveBeenCalledTimes(2)
  })

  it('peekSettings 未加载时为 null，加载后同步返回缓存', async () => {
    expect(peekSettings()).toBeNull()
    getSettingsMock.mockResolvedValue(settingsA)
    await getSettingsCached()
    expect(peekSettings()).toBe(settingsA)
  })

  it('refreshSettings 强制 refetch 并更新缓存；失败保留旧缓存', async () => {
    getSettingsMock.mockResolvedValueOnce(settingsA)
    await getSettingsCached()

    getSettingsMock.mockResolvedValueOnce(settingsB)
    await expect(refreshSettings()).resolves.toBe(settingsB)
    expect(peekSettings()).toBe(settingsB)

    getSettingsMock.mockRejectedValueOnce(new Error('ipc down'))
    await expect(refreshSettings()).rejects.toThrow('ipc down')
    expect(peekSettings()).toBe(settingsB)
  })

  it('refreshSettings 在无缓存时也能拉取并填充缓存（读-改-写冷态路径）', async () => {
    getSettingsMock.mockResolvedValue(settingsA)
    await expect(refreshSettings()).resolves.toBe(settingsA)
    expect(peekSettings()).toBe(settingsA)
  })

  it('saveSettingsCached 成功写通缓存；失败不动缓存', async () => {
    getSettingsMock.mockResolvedValue(settingsA)
    await getSettingsCached()

    saveSettingsMock.mockResolvedValueOnce(settingsB)
    await expect(saveSettingsCached(settingsB)).resolves.toBe(settingsB)
    expect(peekSettings()).toBe(settingsB)

    saveSettingsMock.mockRejectedValueOnce(new Error('save failed'))
    await expect(saveSettingsCached(settingsA)).rejects.toThrow('save failed')
    expect(peekSettings()).toBe(settingsB)
  })

  it('importSettingsCached 成功用导入结果替换缓存', async () => {
    getSettingsMock.mockResolvedValue(settingsA)
    await getSettingsCached()

    importSettingsMock.mockResolvedValueOnce(settingsB)
    await expect(importSettingsCached('/tmp/x.json')).resolves.toBe(settingsB)
    expect(peekSettings()).toBe(settingsB)
  })

  it('setFavoriteModelsCached 成功把新收藏（去重后）补进缓存；失败不动缓存', async () => {
    getSettingsMock.mockResolvedValue(settingsA)
    await getSettingsCached()

    setFavoriteModelsMock.mockResolvedValueOnce(undefined)
    await setFavoriteModelsCached(['p:m', 'p:m', 'p:n'])
    // 与后端 dedup_preserve_order 对齐：按序去重
    expect(peekSettings()?.favoriteModels).toEqual(['p:m', 'p:n'])

    setFavoriteModelsMock.mockRejectedValueOnce(new Error('nope'))
    await expect(setFavoriteModelsCached(['p:other'])).rejects.toThrow('nope')
    expect(peekSettings()?.favoriteModels).toEqual(['p:m', 'p:n'])
  })
})
