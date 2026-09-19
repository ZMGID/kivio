import { act, renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { HistoryItem } from './types'

const {
  loadHistoryFromStorage,
  saveHistoryToStorage,
  lensDeleteHistoryImage,
  lensCommitImageToHistory,
  makeThumbnail,
} = vi.hoisted(() => ({
  loadHistoryFromStorage: vi.fn<() => HistoryItem[]>(),
  saveHistoryToStorage: vi.fn<(items: HistoryItem[]) => void>(),
  lensDeleteHistoryImage: vi.fn<(id: string) => Promise<void>>(() => Promise.resolve()),
  lensCommitImageToHistory: vi.fn<(id: string) => Promise<void>>(() => Promise.resolve()),
  makeThumbnail: vi.fn<(image: string, size: number) => Promise<string>>(),
}))

vi.mock('./history', () => ({
  HISTORY_MAX: 2,
  HISTORY_THUMB_SIZE: 96,
  loadHistoryFromStorage,
  saveHistoryToStorage,
  makeThumbnail,
}))

vi.mock('../api/tauri', () => ({
  api: { lensDeleteHistoryImage, lensCommitImageToHistory },
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
    lensCommitImageToHistory.mockReset().mockResolvedValue(undefined)
    makeThumbnail.mockReset().mockResolvedValue('thumbnail')
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

  it('keeps the newest final snapshot when older thumbnail work finishes later', async () => {
    loadHistoryFromStorage.mockReturnValue([])
    let releaseOld!: (value: string) => void
    let releaseNew!: (value: string) => void
    let commitImage!: () => void
    lensCommitImageToHistory.mockReturnValue(new Promise(resolve => { commitImage = resolve }))
    makeThumbnail.mockReturnValueOnce(new Promise(resolve => { releaseOld = resolve }))
      .mockReturnValueOnce(new Promise(resolve => { releaseNew = resolve }))
    const { result } = renderHook(() => useLensHistory())
    const old = result.current.recordCompleted({ ...item('a'), messages: [{ role: 'assistant', content: 'partial' }] }, 'a')
    const newest = result.current.recordCompleted({ ...item('a'), messages: [{ role: 'assistant', content: 'final error' }] }, 'a')

    await act(async () => { commitImage(); releaseNew('new thumbnail'); await newest })
    expect(result.current.items[0]).toMatchObject({
      imagePreview: 'new thumbnail', messages: [{ role: 'assistant', content: 'final error' }],
    })
    await act(async () => { releaseOld('old thumbnail'); await old })
    expect(result.current.items[0].messages[0].content).toBe('final error')
    expect(lensCommitImageToHistory).toHaveBeenCalledOnce()
  })

  it('retains existing history when image persistence fails and records text without an image', async () => {
    loadHistoryFromStorage.mockReturnValue([item('existing')])
    lensCommitImageToHistory.mockRejectedValueOnce(new Error('disk failed'))
    const log = vi.spyOn(console, 'error').mockImplementation(() => {})
    const { result } = renderHook(() => useLensHistory())
    try {
      await act(async () => result.current.recordCompleted(item('failed'), 'failed'))
      expect(result.current.items.map(entry => entry.id)).toEqual(['existing'])
      expect(lensDeleteHistoryImage).not.toHaveBeenCalled()

      await act(async () => result.current.recordCompleted({ ...item('text'), imagePreview: '' }, ''))
      expect(result.current.items.map(entry => entry.id)).toEqual(['text', 'existing'])
      expect(lensCommitImageToHistory).toHaveBeenCalledOnce()
      expect(result.current.items[0].imagePreview).toBe('')
    } finally { log.mockRestore() }
  })
})
