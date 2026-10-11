// @vitest-environment jsdom
import { render, screen } from '@testing-library/react'
import { expect, it } from 'vitest'
import { BotMessage } from './BotMessage'

it('keeps each historical appearance for the same Bot across a later identity edit', () => {
  const { container } = render(<>
    <BotMessage actor={{ kind: 'bot', id: 'a', name: '旧名称', icon: 'kv1:200', color: '#6E9E80' }} body="旧交付" at={1} />
    <BotMessage actor={{ kind: 'bot', id: 'a', name: '新名称', icon: 'kv1:162', color: '#9A7CB8' }} body="新交付" at={2} />
  </>)
  expect(screen.getByText('旧名称')).toBeVisible()
  expect(screen.getByText('新名称')).toBeVisible()
  const bodies = [...container.querySelectorAll('svg')].map(svg => svg.querySelector('path'))
  expect(bodies.map(body => body?.getAttribute('fill'))).toEqual(['#6E9E80', '#9A7CB8'])
  expect(bodies[0]?.getAttribute('d')).not.toBe(bodies[1]?.getAttribute('d'))
  expect(container.querySelector('[role="button"]')).toBeNull()
})
