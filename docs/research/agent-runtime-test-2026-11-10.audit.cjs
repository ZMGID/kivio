// Read-only audit. Never prints prompts, credentials, tool arguments, or raw responses.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

function modelMessages(conversation) {
  return (conversation.builtin_history?.snapshot?.entries || [])
    .map(entry => entry.kind?.value?.body?.value)
    .filter(message => message?.role);
}

function audit(conversation, usage = [], children = [], messageId) {
  const messages = conversation.messages || [];
  const run = messageId ? messages.find(message => message.id === messageId)
    : messages.filter(message => message.role === 'assistant')
      .reduce((largest, message) => (message.tool_calls?.length || 0) > (largest?.tool_calls?.length || 0) ? message : largest, null);
  assert(run, 'No assistant run with tool calls found');
  const tools = run.tool_calls || [];
  const history = modelMessages(conversation);
  const countBy = (items, key) => Object.fromEntries([...new Set(items.map(key))]
    .map(value => [value, items.filter(item => key(item) === value).length]));
  const goalReads = tools.filter(tool => tool.name === 'get_goal').map(tool => {
    const saved = tool.structured_content?.goalState;
    const result = history.find(message => message.role === 'tool'
      && message.content?.some(part => part.type === 'tool_result' && part.tool_call_id === tool.id));
    const part = result?.content.find(part => part.type === 'tool_result' && part.tool_call_id === tool.id);
    return { round: tool.round, criteriaInStructuredResult: saved?.criteria?.length || 0,
      modelVisibleChars: typeof part?.content === 'string' ? part.content.length : 0,
      modelVisibleResult: typeof part?.content === 'string' && part.content.startsWith('Goal updated: get_goal.')
        ? part.content.split('\n')[0].slice(0, 160) : null };
  });
  const texts = (run.segments || []).filter(segment => segment.kind === 'text');
  const timedTexts = texts.map(segment => {
    const nextTool = tools.find(tool => tool.round >= segment.round);
    return { round: segment.round, time: nextTool?.started_at || run.timestamp,
      degradedNotice: !!segment.text?.startsWith('⚠️') };
  });
  const visibleGaps = timedTexts.slice(1).map((item, i) => ({
    fromRound: timedTexts[i].round, toRound: item.round,
    seconds: item.time - timedTexts[i].time,
  }));
  const runUsage = usage.filter(record => record.messageId === run.id);
  const childPrefix = `subagent-${conversation.id}-`;
  const childUsage = usage.filter(record => record.conversationId?.startsWith(childPrefix));
  const totals = rows => rows.reduce((sum, record) => ({
    requests: sum.requests + 1,
    input: sum.input + (record.inputTokens || 0),
    output: sum.output + (record.outputTokens || 0),
    cached: sum.cached + (record.cachedInputTokens || 0),
  }), { requests: 0, input: 0, output: 0, cached: 0 });
  const calls = tools.map(tool => {
    try { return { tool, args: JSON.parse(tool.arguments) }; }
    catch { return { tool, args: null }; }
  });
  const planningUsage = runUsage.filter(record => record.operation === 'chat_tools_planning');
  const planningTotals = totals(planningUsage);
  return {
    conversationId: conversation.id, messageId: run.id,
    toolCount: tools.length, toolNames: countBy(tools, tool => tool.name),
    errors: tools.filter(tool => tool.status === 'error').map(tool => ({
      round: tool.round, name: tool.name,
      // Shell output may include application data. Keep only its exit classification.
      error: /^(bash|edit|write|read)$/.test(tool.name)
        ? (tool.error || '').split('\n')[0] : tool.error,
    })),
    goalReads,
    staleGoalNotices: history.filter(message => message.role === 'user'
      && message.content?.some(part => part.text?.includes('No acceptance criteria have been initialized'))).length,
    goalInitializations: tools.filter(tool => tool.name === 'initialize_goal_criteria')
      .map(tool => ({ round: tool.round, status: tool.status, error: tool.error })),
    goalCriteria: (conversation.goal_state?.criteria || []).map(criterion => ({
      id: criterion.id, verified: criterion.verified,
      evidenceKind: criterion.evidence_kind, evidenceReference: criterion.evidence_ref,
    })),
    outcome: { streamOutcome: run.stream_outcome, degraded: run.degraded?.kind || null,
      goalStatus: conversation.goal_state?.status },
    goalBookkeeping: conversation.goal_state ? {
      activeRunId: conversation.goal_state.active_run_id,
      inputTokens: conversation.goal_state.input_tokens,
      outputTokens: conversation.goal_state.output_tokens,
      totalTokens: conversation.goal_state.total_tokens,
    } : null,
    visibleGaps,
    broadPostgresKills: calls.filter(({ tool, args }) => tool.name === 'bash'
      && /taskkill\s+\/?\/?[fF]\s+\/?\/?[iI][mM]\s+postgres\.exe/.test(args?.command || ''))
      .map(({ tool }) => tool.round),
    autoBackgrounds: tools.filter(tool => tool.name === 'bash'
      && tool.result_preview?.startsWith('background: true')).map(tool => tool.round),
    usage: { stored: run.usage, mainRequests: totals(runUsage), childRequests: totals(childUsage),
      byOperation: countBy(runUsage, record => record.operation),
      operationTotals: Object.fromEntries([...new Set(runUsage.map(record => record.operation))]
        .map(operation => [operation, totals(runUsage.filter(record => record.operation === operation))])),
      planningCacheKnown: planningUsage.filter(record => record.cachedInputTokens != null).length,
      planningCacheUnknown: planningUsage.filter(record => record.cachedInputTokens == null).length,
      unmergedPlanning: runUsage.length ? { input: planningTotals.input - (run.usage?.input_tokens || 0),
        output: planningTotals.output - (run.usage?.output_tokens || 0) } : null },
    children: children.map(({ name, records }) => {
      const init = records.find(record => record.type === 'session_init');
      const terminal = records.findLast(record => record.type === 'turn_end')?.progress;
      const rows = childUsage.filter(record => record.conversationId === childPrefix + name);
      const last = rows.filter(record => record.operation === 'chat_tools_planning').at(-1);
      return { name, status: terminal?.status, requests: terminal?.requests,
        toolCount: terminal?.toolCount, contextTokens: terminal?.contextTokens,
        contextWindow: terminal?.contextWindow, resolvedEffort: init?.resolvedThinkingLevel,
        recordedEfforts: countBy(rows, record => record.reasoningEffort),
        lastPlanning: last ? { createdAt: last.createdAt, input: last.inputTokens,
          output: last.outputTokens, reasoning: last.reasoningTokens, status: last.status } : null,
        emptyFailureWarning: !!terminal?.preview?.includes('without a final message'),
        systemSnapshots: records.filter(record => record.role === 'system').length,
      };
    }),
  };
}

if (process.argv[2] === '--self-test') {
  const conversation = { id: 'test', messages: [{ role: 'assistant', id: 'reply', timestamp: 10,
    usage: { input_tokens: 10, output_tokens: 1 },
    stream_outcome: 'completed', degraded: { kind: 'empty_response' }, tool_calls: [{
      id: 'call', name: 'get_goal', round: 1, started_at: 1, status: 'success',
      structured_content: { goalState: { criteria: [{ id: 'c1' }] } },
    }] }], builtin_history: { snapshot: { entries: [{ kind: { value: { body: { value: {
      role: 'tool', content: [{ type: 'tool_result', tool_call_id: 'call', content: 'Goal updated: get_goal. Status: Active. Version: 1' }],
    } } } } }] } } };
  const report = audit(conversation, [
    { messageId: 'reply', operation: 'chat_tools_planning', inputTokens: 10, outputTokens: 1 },
    { messageId: 'reply', operation: 'chat_tools_planning', inputTokens: 20, outputTokens: 2 },
    { messageId: 'reply', operation: 'chat_context_compaction', inputTokens: 100, outputTokens: 10 },
  ]);
  assert.equal(report.goalReads[0].criteriaInStructuredResult, 1);
  assert.equal(report.goalReads[0].modelVisibleResult, 'Goal updated: get_goal. Status: Active. Version: 1');
  assert.deepEqual(report.outcome, { streamOutcome: 'completed', degraded: 'empty_response', goalStatus: undefined });
  assert.equal(report.toolCount, 1);
  assert.deepEqual(report.usage.unmergedPlanning, { input: 20, output: 2 });
  assert.equal(report.usage.planningCacheUnknown, 2);
  console.log('audit self-check passed');
} else {
  const conversationPath = process.argv[2];
  if (!conversationPath) throw new Error('Usage: node agent-runtime-test-2026-11-10.audit.cjs <conversation.json> [usage.jsonl] [message-id]');
  const conversation = JSON.parse(fs.readFileSync(conversationPath, 'utf8'));
  const jsonl = file => fs.readFileSync(file, 'utf8').split(/\r?\n/).filter(Boolean).map(JSON.parse);
  const usage = process.argv[3] ? jsonl(process.argv[3]) : [];
  const childDir = path.join(path.dirname(path.dirname(conversationPath)), 'subagents', conversation.id);
  const children = fs.existsSync(childDir) ? fs.readdirSync(childDir).filter(name => name.endsWith('.jsonl'))
    .map(name => ({ name: name.slice(0, -6), records: jsonl(path.join(childDir, name)) })) : [];
  console.log(JSON.stringify(audit(conversation, usage, children, process.argv[4]), null, 2));
}
