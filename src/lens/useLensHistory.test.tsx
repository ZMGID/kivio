import { act, renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { HistoryItem } from './types'

const {
  loadHistoryFromStorage,
  saveHistoryToStorage,
  lensDeleteHistoryImage,
} = vi.hoisted(() => ({
  loadHistoryFromStorage: vi.fn<() => HistoryItem[]>(),
  saveHistoryToStorage: vi.fn<(items: HistoryItem[]) => void>(),
  lensDeleteHistoryImage: vi.fn<(id: string) => Promise<void>>(() => Promise.resolve()),
}))

vi.mock('./history', () => ({
  HISTORY_MAX: 2,
  loadHistoryFromStorage,
  saveHistoryToStorage,
}))

vi.mock('../api/tauri', () => ({
  api: { lensDeleteHistoryImage },
}))

import { useLensHistory } from './useLensHistory'

const item = (id: string, timestamp = 1): HistoryItem => ({
  id,
  imagePreview: `${id}.png`,
  appLabel: id,
  messages: [],
  capturedFrame: null,
  timestamp,
})

describe('useLensHistory', () => {
  beforeEach(() => {
    loadHistoryFromStorage.mockReset()
    saveHistoryToStorage.mockReset()
    lensDeleteHistoryImage.mockReset().mockResolvedValue(undefined)
  })

  it('deduplicates by image identity and owns the bounded history', () => {
    loadHistoryFromStorage.mockReturnValue([item('a'), item('b')])
    const { result } = renderHook(() => useLensHistory())

    act(() => result.current.upsert(item('a', 9)))

    expect(result.current.items.map(({ id, timestamp }) => ({ id, timestamp }))).toEqual([
      { id: 'a', timestamp: 9 },
      { id: 'b', timestamp: 1 },
    ])
  })

  it('persists changes and deletes only images evicted from the owned list', async () => {
    loadHistoryFromStorage.mockReturnValue([item('a'), item('b')])
    const { result } = renderHook(() => useLensHistory())

    act(() => result.current.upsert(item('c')))

    await waitFor(() => expect(lensDeleteHistoryImage).toHaveBeenCalledWith('b'))
    expect(lensDeleteHistoryImage).not.toHaveBeenCalledWith('a')
    expect(saveHistoryToStorage).toHaveBeenLastCalledWith([item('c'), item('a')])
  })
})
