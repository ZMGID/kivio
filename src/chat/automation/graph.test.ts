import { describe, expect, it } from 'vitest'
import {
  FLOW_NODE_GAP_Y,
  FLOW_NODE_MIN_GAP,
  canConnect,
  connectNodes,
  createFlowNode,
  ensureNodeSpacing,
  nodeVisibleBounds,
  flowEdgeFromConnection,
  pickAppendSource,
  pruneDanglingBranchEdges,
} from './graph'
import { slotAttachPosition } from './agentModel'

function node(id: string, type: 'trigger.manual' | 'action.agent' | 'action.notify' | 'logic.if', x = 0) {
  return createFlowNode(type, { label: id }, { x, y: 0 })
}

describe('automation graph', () => {
  function expectSpacing(nodes: Parameters<typeof ensureNodeSpacing>[0]) {
    for (let i = 0; i < nodes.length; i++) {
      for (let j = i + 1; j < nodes.length; j++) {
        const a = nodeVisibleBounds(nodes[i])
        const b = nodeVisibleBounds(nodes[j])
        expect(a.right + FLOW_NODE_MIN_GAP <= b.left || b.right + FLOW_NODE_MIN_GAP <= a.left
          || a.bottom + FLOW_NODE_MIN_GAP <= b.top || b.bottom + FLOW_NODE_MIN_GAP <= a.top).toBe(true)
      }
    }
  }

  it('separates imported piles including Agent captions and all four slot nodes', () => {
    const nodes = ['action.agent', 'action.notify', 'agent.runtime', 'agent.context', 'agent.tool', 'agent.skill'].map((type, i) => ({
      id: String(i), type, position: { x: 0, y: 0 },
    }))
    const spaced = ensureNodeSpacing(nodes)
    expectSpacing(spaced)
    expect(spaced[0]).toBe(nodes[0])
    expect(ensureNodeSpacing(spaced)).toBe(spaced)
    expect(nodes.every((item) => item.position.x === 0 && item.position.y === 0)).toBe(true)
  })

  it('moves the dragged node while leaving its stationary neighbor fixed', () => {
    const dragged = { id: 'dragged', type: 'action.agent', position: { x: 0, y: 0 } }
    const fixed = { id: 'fixed', type: 'action.notify', position: { x: 30, y: 30 } }
    const spaced = ensureNodeSpacing([dragged, fixed], new Set(['dragged']))
    expect(spaced[1]).toBe(fixed)
    expect(spaced[0].position).not.toEqual(dragged.position)
    expectSpacing(spaced)
  })

  it('enforces a gap even when cards do not overlap but captions touch', () => {
    const nodes = [
      { id: 'a', type: 'action.notify', position: { x: 0, y: 0 } },
      { id: 'b', type: 'action.notify', position: { x: 180, y: 0 } },
    ]
    expect(ensureNodeSpacing(nodes)).not.toBe(nodes)
    expectSpacing(ensureNodeSpacing(nodes))
  })

  it('spaces default satellite positions and repeated tool slots', () => {
    const agent = { id: 'a', type: 'action.agent', position: { x: 0, y: 0 } }
    const slots = (['runtime', 'context', 'tool', 'skill'] as const).map((slot) => ({
      id: slot, type: `agent.${slot}`, position: slotAttachPosition(agent.position, slot),
    }))
    expectSpacing([agent, ...slots, { id: 'tool-2', type: 'agent.tool', position: slotAttachPosition(agent.position, 'tool', 1) }])
  })

  it('separates every node in a dense mixed-size graph', () => {
    const nodes = Array.from({ length: 60 }, (_, i) => ({
      id: String(i), type: i % 3 ? 'action.notify' : 'action.agent',
      position: { x: (i % 8) * 40, y: Math.floor(i / 8) * 24 },
    }))
    expectSpacing(ensureNodeSpacing(nodes))
  })
  it('允许一连多，目标仍只能有一个入口', () => {
    const trigger = { ...node('t', 'trigger.manual'), id: 't' }
    const agent = { ...node('a', 'action.agent', 200), id: 'a' }
    const notify = { ...node('n', 'action.notify', 400), id: 'n' }
    const nodes = [trigger, agent, notify]
    expect(canConnect('t', 'a', nodes, [])).toBe(true)
    expect(canConnect('a', 't', nodes, [])).toBe(false)
    const edges = [connectNodes('t', 'a')]
    expect(canConnect('t', 'n', nodes, edges)).toBe(true)
    expect(canConnect('a', 'n', nodes, edges)).toBe(true)
    expect(canConnect('n', 'a', nodes, [...edges, connectNodes('a', 'n')])).toBe(false)
    expect(canConnect('t', 'a', nodes, edges)).toBe(false)
  })

  it('拒绝把下游接回上游形成环', () => {
    const a = { ...node('a', 'action.notify', 200), id: 'a' }
    const b = { ...node('b', 'action.notify', 400), id: 'b' }
    const nodes = [a, b]
    expect(canConnect('a', 'b', nodes, [])).toBe(true)
    expect(canConnect('b', 'a', nodes, [connectNodes('a', 'b')])).toBe(false)
  })

  it('Switch 每个出口只能连一条边', () => {
    const trigger = { ...node('t', 'trigger.manual'), id: 't' }
    const sw = createFlowNode('logic.switch', {
      label: 's',
      switch: { cases: [{ id: '1', op: 'equals', value: 'a' }] },
    }, { x: 200, y: 0 })
    sw.id = 's'
    const a = { ...node('a', 'action.notify', 400), id: 'a' }
    const b = { ...node('b', 'action.notify', 400), id: 'b' }
    const nodes = [trigger, sw, a, b]
    const after = [connectNodes('t', 's')]
    expect(canConnect('s', 'a', nodes, after, '1')).toBe(true)
    const withOne = [...after, connectNodes('s', 'a', '1')]
    expect(canConnect('s', 'a', nodes, withOne, '1')).toBe(false)
    expect(canConnect('s', 'b', nodes, withOne, 'default')).toBe(true)
  })

  it('If 节点允许 true/false 两个出口', () => {
    const trigger = { ...node('t', 'trigger.manual'), id: 't' }
    const iff = { ...node('i', 'logic.if', 200), id: 'i' }
    const yes = { ...node('y', 'action.notify', 400), id: 'y' }
    const no = { ...node('n', 'action.notify', 400), id: 'n' }
    const nodes = [trigger, iff, yes, no]
    expect(canConnect('t', 'i', nodes, [])).toBe(true)
    const afterTrigger = [connectNodes('t', 'i')]
    expect(canConnect('i', 'y', nodes, afterTrigger, 'true')).toBe(true)
    const withTrue = [...afterTrigger, connectNodes('i', 'y', 'true')]
    expect(canConnect('i', 'y', nodes, withTrue, 'true')).toBe(false)
    expect(canConnect('i', 'n', nodes, withTrue, 'false')).toBe(true)
  })

  it('多个触发器可以接到同一步', () => {
    const manual = { ...node('m', 'trigger.manual'), id: 'm' }
    const schedule = createFlowNode('trigger.schedule', { label: 's' }, { x: 0, y: FLOW_NODE_GAP_Y })
    schedule.id = 's'
    const agent = { ...node('a', 'action.agent', 200), id: 'a' }
    const notify = { ...node('n', 'action.notify', 400), id: 'n' }
    const nodes = [manual, schedule, agent, notify]
    expect(canConnect('m', 'a', nodes, [])).toBe(true)
    expect(canConnect('s', 'a', nodes, [connectNodes('m', 'a')])).toBe(true)
    expect(canConnect('s', 'n', nodes, [connectNodes('a', 'n')])).toBe(false)
  })

  it('添加下一步接到还能出边的节点', () => {
    const trigger = { ...node('t', 'trigger.manual'), id: 't' }
    const agent = { ...node('a', 'action.agent'), id: 'a' }
    const nodes = [trigger, agent]
    expect(pickAppendSource(nodes, [])).toEqual({ nodeId: 'a' })
    expect(pickAppendSource(nodes, [connectNodes('t', 'a')])).toEqual({ nodeId: 'a' })
    expect(pickAppendSource(nodes, [connectNodes('t', 'a')], 't')).toEqual({ nodeId: 'a' })
    const notify = { ...node('n', 'action.notify'), id: 'n' }
    expect(pickAppendSource(
      [...nodes, notify],
      [connectNodes('t', 'a'), connectNodes('a', 'n')],
    )).toEqual({ nodeId: 'n' })
  })

  it('slot nodes plug into Agent without taking the main-flow input', () => {
    const trigger = { ...node('t', 'trigger.manual'), id: 't' }
    const agent = { ...node('a', 'action.agent', 200), id: 'a' }
    const runtime = createFlowNode('agent.runtime', { label: 'r' }, { x: 200, y: 200 })
    runtime.id = 'r'
    const nodes = [trigger, agent, runtime]
    expect(canConnect('t', 'a', nodes, [])).toBe(true)
    expect(canConnect('r', 'a', nodes, [connectNodes('t', 'a')], 'slot', 'runtime')).toBe(true)
    expect(canConnect('r', 'a', nodes, [connectNodes('t', 'a')], 'slot')).toBe(true)
    expect(canConnect('r', 'a', nodes, [connectNodes('t', 'a')], 'slot', 'context')).toBe(false)
    const plugged = [connectNodes('t', 'a'), connectNodes('r', 'a', 'slot', 'runtime')]
    expect(canConnect('r', 'a', nodes, plugged, 'slot', 'runtime')).toBe(false)
    expect(pickAppendSource(nodes, plugged)).toEqual({ nodeId: 'a' })
    const remapped = flowEdgeFromConnection('agent.runtime', 'r', 'a', 'slot', null)
    expect(remapped.targetHandle).toBe('runtime')
    expect(remapped.sourceHandle).toBe('slot')
    expect(canConnect('t', 'a', nodes, [remapped])).toBe(true)
  })

  it('删除 switch case 时清掉该口上的边', () => {
    const sw = createFlowNode('logic.switch', {
      label: 's',
      switch: { cases: [{ id: '1', op: 'equals', value: 'a' }] },
    }, { x: 0, y: 0 })
    sw.id = 's'
    const a = { ...node('a', 'action.notify'), id: 'a' }
    const b = { ...node('b', 'action.notify'), id: 'b' }
    const edges = [
      connectNodes('s', 'a', '1'),
      connectNodes('s', 'b', 'default'),
    ]
    const kept = pruneDanglingBranchEdges(
      [{ ...sw, data: { ...sw.data, switch: { cases: [] } } }, a, b],
      edges,
    )
    expect(kept.map((edge) => edge.sourceHandle)).toEqual(['default'])
  })
})
