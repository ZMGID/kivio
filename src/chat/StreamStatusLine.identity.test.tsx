// @vitest-environment jsdom
import { render, waitFor } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import { StreamStatusLine } from './StreamStatusLine'

vi.mock('./conversationTransitionStore', () => ({ useConversationTransition: () => ({ loading: false }) }))

it('uses the selected Bot appearance above the composer, updates edits, and preserves the Agent mascot', async () => {
  const { container, rerender } = render(<StreamStatusLine active={false} botAppearance={{ icon: 'kv1:213', color: '#6E9E80' }} />)
  const body = () => container.querySelector('svg path')!
  expect(body().getAttribute('fill')).toBe('#6E9E80')
  expect(container.querySelectorAll('svg path')).toHaveLength(6)
  const cloud = body().getAttribute('d')
  rerender(<StreamStatusLine active={false} botAppearance={{ icon: 'kv1:162', color: '#9A7CB8' }} />)
  await waitFor(() => expect(body().getAttribute('fill')).toBe('#9A7CB8'))
  expect(body().getAttribute('d')).not.toBe(cloud)
  rerender(<StreamStatusLine active={false} />)
  await waitFor(() => expect(body().getAttribute('fill')).toBe('#1d6bf0'))
  expect(container.querySelectorAll('svg path')).toHaveLength(3)
})
