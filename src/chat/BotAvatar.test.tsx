// @vitest-environment jsdom
import { act, render, screen } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { BotAvatar } from './BotAvatar'

afterEach(() => vi.useRealTimers())

it('celebrates a new successful result briefly, but never celebrates restored history', () => {
  vi.useFakeTimers()
  const { rerender } = render(<BotAvatar mood="idle" label="安静" successKey="old" />)
  expect(screen.getByRole('img')).toHaveAccessibleName('安静')
  rerender(<BotAvatar mood="work" label="工作中" successKey="old" />)
  rerender(<BotAvatar mood="idle" label="安静" successKey="new" />)
  expect(screen.getByRole('img')).toHaveAccessibleName('刚刚完成')
  act(() => { vi.advanceTimersByTime(2700) })
  expect(screen.getByRole('img')).toHaveAccessibleName('安静')
})

it('a changed success snapshot or a waiting state cannot strand a completion expression', () => {
  vi.useFakeTimers()
  const { rerender } = render(<BotAvatar mood="work" label="工作中" />)
  rerender(<BotAvatar mood="idle" label="安静" successKey="one" />)
  expect(screen.getByRole('img')).toHaveAccessibleName('刚刚完成')
  rerender(<BotAvatar mood="idle" label="安静" successKey="two" />)
  expect(screen.getByRole('img')).toHaveAccessibleName('安静')
  rerender(<BotAvatar mood="wait" label="等你处理" successKey="two" />)
  expect(screen.getByRole('img')).toHaveAccessibleName('等你处理')
})

it('waits for presentation settlement and only celebrates added success events, never a removed success after cancellation', () => {
  vi.useFakeTimers()
  const { rerender } = render(<BotAvatar mood="idle" label="安静" successKey="private-old|peer-old" />)
  rerender(<BotAvatar mood="work" label="工作中" successKey="private-old|peer-old" />)
  rerender(<BotAvatar mood="work" label="工作中" successKey="private-old|peer-new" />)
  expect(screen.getByRole('img')).toHaveAccessibleName('工作中')
  rerender(<BotAvatar mood="idle" label="安静" successKey="private-old|peer-new" />)
  expect(screen.getByRole('img')).toHaveAccessibleName('刚刚完成')
  rerender(<BotAvatar mood="work" label="工作中" successKey="private-old|peer-new" />)
  rerender(<BotAvatar mood="idle" label="安静" successKey="|peer-new" />)
  expect(screen.getByRole('img')).toHaveAccessibleName('安静')
})
