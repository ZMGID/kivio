import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import type { Settings } from '../api/tauri'
import { LangContext } from '../components/i18n'
import { makeProvider } from '../settings/tabs/testFixtures'

const mocks = vi.hoisted(() => ({ settings: {} as Settings, roles: vi.fn() }))

vi.mock('../api/tauri', async importOriginal => ({
  ...await importOriginal<typeof import('../api/tauri')>(),
  api: {
    onMcpServerState: async () => () => {},
    chatMcpServerStatus: async () => ({ state: { kind: 'disconnected' } }),
    chatSubagentRoles: mocks.roles,
  },
}))
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
  mocks.roles.mockReset().mockResolvedValue([])
  mocks.settings = {
    chatTools: {
      servers: [],
      taskAgentAdvisor: { reviewer: 'provider/model' },
      taskAgentModelOverrides: { reviewer: ['provider/model'] },
    },
  } as unknown as Settings
})

it('manages roles in the current conversation and preserves sibling settings', async () => {
  mocks.roles.mockResolvedValue([
    { id: 'scout', name: 'scout', source: 'builtin', description: 'Research', unsupported: [] },
    { id: 'auditor', name: 'auditor', source: 'project', description: 'Audit', unsupported: [] },
  ])
  mocks.settings.providers = [
    makeProvider({ id: 'p1', name: 'Provider 1', enabledModels: ['new-model', 'backup'] }),
    makeProvider({ id: 'p2', enabled: false, enabledModels: ['disabled-model'] }),
    makeProvider({ id: 'p3', enabledModels: ['unauthed-model'], request: { oauth: { provider: 'codex' } } }),
  ]
  mocks.settings.chatTools.taskDisabledAgents = ['auditor']
  mocks.settings.chatTools.taskAgentModelOverrides = { scout: ['old/model', 'p1/backup'], reviewer: ['provider/model'] }
  render(<LangContext.Provider value="en"><McpCenter conversationId="conv-role-test" /></LangContext.Provider>)
  fireEvent.click(screen.getByRole('button', { name: 'Advanced' }))
  const enableScout = await screen.findByRole('switch', { name: 'Enable scout' })
  expect(mocks.roles).toHaveBeenCalledWith('conv-role-test')
  fireEvent.click(enableScout)
  await waitFor(() => expect(mocks.settings.chatTools.taskDisabledAgents).toEqual(['auditor', 'scout']))
  expect(enableScout).toHaveAttribute('aria-checked', 'false')
  expect(screen.getByRole('button', { name: 'scout · Model' })).toBeDisabled()
  fireEvent.click(enableScout)
  await waitFor(() => expect(mocks.settings.chatTools.taskDisabledAgents).toEqual(['auditor']))

  fireEvent.click(screen.getByRole('button', { name: 'scout · Model' }))
  expect(screen.queryByRole('option', { name: /disabled-model|unauthed-model/ })).not.toBeInTheDocument()
  fireEvent.click(screen.getByRole('option', { name: 'new-model · Provider 1' }))
  await waitFor(() => expect(mocks.settings.chatTools.taskAgentModelOverrides).toEqual({ scout: ['p1/new-model', 'p1/backup'], reviewer: ['provider/model'] }))

  fireEvent.click(screen.getByRole('button', { name: 'scout · Advisor' }))
  fireEvent.click(screen.getByRole('option', { name: 'Off' }))
  await waitFor(() => expect(mocks.settings.chatTools.taskAgentAdvisor).toEqual({ scout: 'off', reviewer: 'provider/model' }))

  fireEvent.click(screen.getByRole('button', { name: 'scout · Prewalk' }))
  fireEvent.click(screen.getByRole('option', { name: 'On · default model' }))
  await waitFor(() => expect(mocks.settings.chatTools.taskAgentPrewalk).toEqual({ scout: 'on' }))
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
