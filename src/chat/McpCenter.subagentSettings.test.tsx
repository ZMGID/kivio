import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import type { Settings } from '../api/tauri'
import { LangContext } from '../components/i18n'

const mocks = vi.hoisted(() => ({ settings: {} as Settings }))

vi.mock('../api/tauri', () => ({ api: {
  onMcpServerState: async () => () => {},
  chatMcpServerStatus: async () => ({ state: { kind: 'disconnected' } }),
} }))
vi.mock('../api/settingsCache', () => ({
  peekSettings: () => mocks.settings,
  refreshSettings: async () => mocks.settings,
  subscribeSettings: () => () => {},
  updateSettingsCached: async (update: (settings: Settings) => Settings) => {
    mocks.settings = update(mocks.settings)
    return mocks.settings
  },
}))
vi.mock('./McpRegistryBrowser', () => ({ McpRegistryBrowser: () => null }))

import { McpCenter } from './McpCenter'

beforeEach(() => {
  mocks.settings = {
    chatTools: {
      servers: [],
      taskAgentAdvisor: { reviewer: 'provider/model' },
      taskAgentModelOverrides: { reviewer: ['provider/model'] },
    },
  } as unknown as Settings
})

it('saves subagent capabilities, model permission, effort ceiling, and model scope', async () => {
  render(<LangContext.Provider value="en"><McpCenter /></LangContext.Provider>)
  fireEvent.click(screen.getByRole('button', { name: 'Advanced' }))

  const toggles = [
    ['Allow task files', 'taskFileEnabled'],
    ['Structured results', 'taskStructured'],
    ['Allow per-task effort', 'taskEnableEffort'],
    ['Allow workspace isolation', 'taskIsolationEnabled'],
    ['Plan before implementation', 'taskPrewalk'],
    ['Use fresh workpool agents', 'taskEvalWorkpoolFreshAgents'],
  ] as const
  for (const [label, key] of toggles) {
    const toggle = await screen.findByRole('switch', { name: label })
    expect(toggle).toHaveAttribute('aria-checked', 'false')
    fireEvent.click(toggle)
    await waitFor(() => expect(mocks.settings.chatTools[key]).toBe(true))
    expect(toggle).toHaveAttribute('aria-checked', 'true')
  }

  fireEvent.click(screen.getByRole('button', { name: 'Per-task model selection' }))
  fireEvent.click(screen.getByRole('option', { name: 'Eval only' }))
  await waitFor(() => expect(mocks.settings.chatTools.taskPerCallModel).toBe('eval'))

  fireEvent.click(screen.getByRole('button', { name: 'Maximum reasoning effort' }))
  fireEvent.click(screen.getByRole('option', { name: 'medium' }))
  await waitFor(() => expect(mocks.settings.chatTools.taskMaxEffort).toBe('medium'))

  const allow = screen.getByRole('textbox', { name: 'Allowed model scope' })
  fireEvent.change(allow, { target: { value: ' p1/model-a, p2/model-b ' } })
  expect(mocks.settings.chatTools.taskPerCallModelAllow).toBeUndefined()
  fireEvent.blur(allow)
  await waitFor(() => expect(mocks.settings.chatTools.taskPerCallModelAllow).toEqual(['p1/model-a', 'p2/model-b']))
  expect(allow).toHaveValue('p1/model-a, p2/model-b')
  expect(mocks.settings.chatTools.taskAgentAdvisor).toEqual({ reviewer: 'provider/model' })
  expect(mocks.settings.chatTools.taskAgentModelOverrides).toEqual({ reviewer: ['provider/model'] })

  fireEvent.change(allow, { target: { value: '' } })
  fireEvent.blur(allow)
  await waitFor(() => expect(mocks.settings.chatTools.taskPerCallModelAllow).toEqual([]))
  fireEvent.click(screen.getByRole('button', { name: 'Maximum reasoning effort' }))
  fireEvent.click(screen.getByRole('option', { name: 'No limit' }))
  await waitFor(() => expect(mocks.settings.chatTools.taskMaxEffort).toBeNull())
  fireEvent.click(screen.getByRole('switch', { name: 'Allow task files' }))
  await waitFor(() => expect(mocks.settings.chatTools.taskFileEnabled).toBe(false))
})

it('restores saved values and offers Chinese labels', async () => {
  mocks.settings.chatTools.taskFileEnabled = true
  mocks.settings.chatTools.taskPerCallModel = 'on'
  mocks.settings.chatTools.taskMaxEffort = 'high'
  mocks.settings.chatTools.taskPerCallModelAllow = ['p1/m1']
  render(<LangContext.Provider value="zh"><McpCenter /></LangContext.Provider>)
  fireEvent.click(screen.getByRole('button', { name: '高级设置' }))
  expect(await screen.findByRole('switch', { name: '允许从文件读取任务' })).toHaveAttribute('aria-checked', 'true')
  expect(screen.getByRole('button', { name: '按任务选择模型' })).toHaveTextContent('所有任务')
  expect(screen.getByRole('button', { name: '最高推理强度' })).toHaveTextContent('high')
  expect(screen.getByRole('textbox', { name: '可选模型范围' })).toHaveValue('p1/m1')
})
