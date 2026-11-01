import { describe, expect, it } from 'vitest'
import { Eye, ShieldCheck } from 'lucide-react'
import { deriveDshPresetModes, derivePermissionModes } from './permissionModes'
import type { AgentRuntimeConfig, DetectedExternalAgent } from './types'

const builtinRuntime: AgentRuntimeConfig = { kind: 'builtin' }
const chatRuntime: AgentRuntimeConfig = { kind: 'chat' }

function externalRuntime(id: string, sandbox?: string | null): AgentRuntimeConfig {
  return { kind: 'external', externalAgentId: id, externalSandbox: sandbox ?? null }
}

// 后端 detection::sandbox_options_for 的档位表（claude 四档 / codex 三档 / opencode 两档）。
const agents: DetectedExternalAgent[] = [
  {
    id: 'claude',
    name: 'Claude Code',
    available: true,
    models: [],
    sandboxOptions: [
      { id: 'plan', label: '计划 (只读)' },
      { id: 'default', label: '每次确认' },
      { id: 'acceptEdits', label: '接受编辑' },
      { id: 'bypassPermissions', label: '完全 (默认)' },
    ],
  },
  {
    id: 'codex',
    name: 'Codex',
    available: true,
    models: [],
    sandboxOptions: [
      { id: 'read-only', label: '只读' },
      { id: 'workspace-write', label: '工作区写 (默认)' },
      { id: 'danger-full-access', label: '完全' },
    ],
  },
  {
    id: 'opencode', name: 'OpenCode', available: true, models: [],
    sandboxOptions: [
      { id: 'build', label: '构建 (默认)' },
      { id: 'plan', label: '计划 (只读)' },
    ],
  },
]

describe('derivePermissionModes（底栏模式胶囊）', () => {
  it('maps detected Claude sandbox options', () => {
    const { options } = derivePermissionModes({
      target: 'composer',
      agentRuntime: externalRuntime('claude'),
      agents,
    })
    expect(options.map((o) => o.value)).toEqual(agents[0].sandboxOptions!.map((o) => o.id))
    expect(options.map((o) => o.label)).toEqual(agents[0].sandboxOptions!.map((o) => o.label))
  })

  it('maps detected Codex sandbox options', () => {
    const { options } = derivePermissionModes({
      target: 'composer',
      agentRuntime: externalRuntime('codex'),
      agents,
    })
    expect(options.map((o) => o.value)).toEqual(agents[1].sandboxOptions!.map((o) => o.id))
  })

  it('opencode uses build by default and the same read-only eye and neutral tone as Claude plan', () => {
    const { options, current } = derivePermissionModes({
      target: 'composer',
      agentRuntime: externalRuntime('opencode'),
      agents,
    })
    const claude = derivePermissionModes({ target: 'composer', agentRuntime: externalRuntime('claude'), agents })
    expect(current).toBe('build')
    expect(options.map(({ value, label }) => ({ value, label }))).toEqual(
      agents[2].sandboxOptions!.map(({ id, label }) => ({ value: id, label })),
    )
    expect(options[0].icon).toBe(ShieldCheck)
    expect(options[1].icon).toBe(Eye)
    expect(options[1].tone).toBe(claude.options[0].tone)
    expect(derivePermissionModes({
      target: 'composer', agentRuntime: externalRuntime('opencode', 'plan'), agents,
    }).current).toBe('plan')
  })

  it('还没探测到 agent 列表时也是空表，不会退回 Kivio 三档', () => {
    const { options } = derivePermissionModes({
      target: 'composer',
      agentRuntime: externalRuntime('claude'),
      agents: [],
    })
    expect(options).toEqual([])
  })

  it('未显式选过档位时跟随 CLI 标了「默认」的那档', () => {
    expect(derivePermissionModes({
      target: 'composer',
      agentRuntime: externalRuntime('claude'),
      agents,
    }).current).toBe('bypassPermissions')
    expect(derivePermissionModes({
      target: 'composer',
      agentRuntime: externalRuntime('codex'),
      agents,
    }).current).toBe('workspace-write')
  })

  it('显式选过就用选中的那档', () => {
    expect(derivePermissionModes({
      target: 'composer',
      agentRuntime: externalRuntime('claude', 'plan'),
      agents,
    }).current).toBe('plan')
  })

  it('内置 Agent 会话给 Kivio 四档', () => {
    const { options, current } = derivePermissionModes({
      target: 'composer',
      agentRuntime: builtinRuntime,
      agents,
      agentPlanMode: 'plan',
    })
    expect(options.map((o) => o.value)).toEqual(['act', 'goal', 'plan', 'orchestrate'])
    expect(options.map((o) => o.label)).toEqual(['Act', 'Goal', 'Plan', 'Orchestrate'])
    expect(current).toBe('plan')
  })

  it('未完成 Goal 在底栏保持选中，同时后端策略仍为 Act', () => {
    expect(derivePermissionModes({
      target: 'composer',
      agentRuntime: builtinRuntime,
      agentPlanMode: 'act',
      goalActive: true,
    }).current).toBe('goal')
  })

  it('内置会话没有档位状态时回落 act', () => {
    expect(derivePermissionModes({
      target: 'composer',
      agentRuntime: builtinRuntime,
      agentPlanMode: null,
    }).current).toBe('act')
  })

  it('Kivio Chat 运行时底栏无 Act/Plan/Orchestrate（独立 runtime）', () => {
    expect(derivePermissionModes({
      target: 'composer',
      agentRuntime: chatRuntime,
      agents,
      agentPlanMode: 'act',
    }).options).toEqual([])
  })
})

describe('derivePermissionModes（顶栏权限按钮）', () => {
  it('内置会话给工具审批策略三档', () => {
    const { options, current } = derivePermissionModes({
      target: 'titlebar',
      agentRuntime: builtinRuntime,
      approvalPolicy: 'auto',
    })
    expect(options.map((o) => o.value)).toEqual([
      'always_confirm',
      'readonly_auto_sensitive_confirm',
      'auto',
    ])
    expect(current).toBe('auto')
  })

  it('没有显式策略时回落到「敏感确认」', () => {
    expect(derivePermissionModes({
      target: 'titlebar',
      agentRuntime: builtinRuntime,
    }).current).toBe('readonly_auto_sensitive_confirm')
  })

  it('本地 CLI 会话给空表（顶栏隐藏，档位归底栏胶囊一处管）', () => {
    for (const id of ['claude', 'codex', 'opencode']) {
      expect(derivePermissionModes({
        target: 'titlebar',
        agentRuntime: externalRuntime(id),
        agents,
      }).options).toEqual([])
    }
  })

  it('Kivio Chat 顶栏也无档位表', () => {
    expect(derivePermissionModes({
      target: 'titlebar',
      agentRuntime: chatRuntime,
    }).options).toEqual([])
  })
})

describe('deriveDshPresetModes（底栏 Agent 模式胶囊）', () => {
  it('uses the default preset unless a preset is selected', () => {
    expect(deriveDshPresetModes(externalRuntime('dsh')).current).toBe('standard')
    expect(deriveDshPresetModes({
      kind: 'external',
      externalAgentId: 'dsh',
      externalAgentPreset: 'minimal',
    }).current).toBe('minimal')
  })

  it('appends a custom preset and skips a duplicate built-in preset', () => {
    const builtins = deriveDshPresetModes(externalRuntime('dsh')).options
    const modes = deriveDshPresetModes(externalRuntime('dsh'), [
      { id: 'code-review', label: '代码审查', description: '只读评审' },
      { id: builtins[0].value, label: '应被跳过' },
    ])
    expect(modes.options.map((o) => o.value)).toEqual([
      ...builtins.map((o) => o.value),
      'code-review',
    ])
    expect(modes.options.at(-1)).toMatchObject({
      value: 'code-review',
      label: '代码审查',
      description: '只读评审',
    })
  })

  it('当前值是尚未扫到的自定义 id 时仍显示该项，不回落 standard', () => {
    const modes = deriveDshPresetModes({
      kind: 'external',
      externalAgentId: 'dsh',
      externalAgentPreset: 'my-writer',
    })
    expect(modes.current).toBe('my-writer')
    expect(modes.options.map((o) => o.value)).toContain('my-writer')
  })

  it('非 dsh 会话不显示 Agent 模式胶囊', () => {
    expect(deriveDshPresetModes(externalRuntime('claude')).options).toEqual([])
    expect(deriveDshPresetModes(builtinRuntime).options).toEqual([])
  })
})
