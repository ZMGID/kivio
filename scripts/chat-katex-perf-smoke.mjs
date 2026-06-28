import { createRequire } from 'node:module'

// Parameterized KaTeX regression smoke.
//
// Required: serve the app first, for example:
//   npm run preview -- --port 5714 --strictPort
//
// Default user-repro case:
//   npx --yes --package=playwright bash -lc 'export NODE_PATH=$(dirname "$(dirname "$(which playwright)")"); node scripts/chat-katex-perf-smoke.mjs'
//
// Stress matrix example:
//   KIVIO_PERF_BASE_URL=http://localhost:5714/ \
//   KIVIO_PERF_FORMULA_COUNTS=1,3,8 \
//   KIVIO_PERF_RUNS=3 \
//   KIVIO_PERF_SIDEBAR_ITERATIONS=30 \
//   KIVIO_PERF_SEARCH_ITERATIONS=20 \
//   KIVIO_PERF_THEME_ITERATIONS=30 \
//   npx --yes --package=playwright bash -lc 'export NODE_PATH=$(dirname "$(dirname "$(which playwright)")"); node scripts/chat-katex-perf-smoke.mjs'
//
// Supported env:
// - KIVIO_PERF_BASE_URL, KIVIO_PERF_BROWSERS=chromium,webkit
// - KIVIO_PERF_FORMULA_COUNTS=1,3,8, KIVIO_PERF_RUNS=3
// - KIVIO_PERF_SIDEBAR_ITERATIONS, KIVIO_PERF_SEARCH_ITERATIONS, KIVIO_PERF_THEME_ITERATIONS
// - KIVIO_PERF_VIEWPORT=1280x800, KIVIO_PERF_DEVICE_SCALE_FACTOR=2
// - KIVIO_PERF_WEBKIT_FORMULA_TO_PLAIN_RATIO
// - KIVIO_PERF_WEBKIT_MAX_SIDEBAR_MS, KIVIO_PERF_WEBKIT_MAX_SEARCH_MS, KIVIO_PERF_WEBKIT_MAX_THEME_MS

const require = createRequire(import.meta.url)
let playwright
try {
  playwright = require('playwright')
} catch {
  console.error('Playwright is required. Run with:')
  console.error("  npx --yes --package=playwright bash -lc 'export NODE_PATH=$(dirname \"$(dirname \"$(which playwright)\")\"); node scripts/chat-katex-perf-smoke.mjs'")
  process.exit(1)
}

const { chromium, webkit } = playwright
const storageKey = 'kivio-chat-dev-conversations'
const lastRouteKey = 'kivio-chat-last-route'
const now = Math.floor(Date.now() / 1000)
const browserTypes = { chromium, webkit }

function parsePositiveInt(value, fallback) {
  const parsed = Number.parseInt(String(value ?? ''), 10)
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback
}

function parsePositiveNumber(value, fallback) {
  const parsed = Number.parseFloat(String(value ?? ''))
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback
}

function parseFormulaCounts(value) {
  const raw = String(value ?? '1')
    .split(',')
    .map((part) => Number.parseInt(part.trim(), 10))
    .filter((item) => Number.isFinite(item) && item > 0)
  return raw.length > 0 ? [...new Set(raw)] : [1]
}

function parseBrowsers(value) {
  const raw = String(value ?? 'chromium,webkit')
    .split(',')
    .map((part) => part.trim().toLowerCase())
    .filter(Boolean)
  const selected = raw.length > 0 ? raw : ['chromium', 'webkit']
  const unknown = selected.filter((name) => !(name in browserTypes))
  if (unknown.length > 0) {
    console.error(`Unknown browser(s): ${unknown.join(', ')}. Use KIVIO_PERF_BROWSERS=chromium,webkit`)
    process.exit(1)
  }
  return [...new Set(selected)]
}

function parseViewport(value) {
  const match = /^(\d+)x(\d+)$/i.exec(String(value ?? '').trim())
  if (!match) return { width: 1280, height: 800 }
  return {
    width: Math.max(320, Number.parseInt(match[1], 10)),
    height: Math.max(320, Number.parseInt(match[2], 10)),
  }
}

const config = {
  baseUrl: process.env.KIVIO_PERF_BASE_URL ?? 'http://localhost:5714/',
  browsers: parseBrowsers(process.env.KIVIO_PERF_BROWSERS),
  formulaCounts: parseFormulaCounts(process.env.KIVIO_PERF_FORMULA_COUNTS),
  runs: parsePositiveInt(process.env.KIVIO_PERF_RUNS, 3),
  iterations: {
    sidebar: parsePositiveInt(process.env.KIVIO_PERF_SIDEBAR_ITERATIONS, 28),
    search: parsePositiveInt(process.env.KIVIO_PERF_SEARCH_ITERATIONS, 18),
    theme: parsePositiveInt(process.env.KIVIO_PERF_THEME_ITERATIONS, 28),
  },
  viewport: parseViewport(process.env.KIVIO_PERF_VIEWPORT),
  deviceScaleFactor: parsePositiveNumber(process.env.KIVIO_PERF_DEVICE_SCALE_FACTOR, 2),
  thresholds: {
    webkitFormulaToPlainRatio: parsePositiveNumber(process.env.KIVIO_PERF_WEBKIT_FORMULA_TO_PLAIN_RATIO, 2.8),
    webkitMaxBlockingMsByOp: {
      sidebar_class_toggle: parsePositiveNumber(process.env.KIVIO_PERF_WEBKIT_MAX_SIDEBAR_MS, 20),
      search_dialog_toggle: parsePositiveNumber(process.env.KIVIO_PERF_WEBKIT_MAX_SEARCH_MS, 60),
      theme_color_toggle: parsePositiveNumber(process.env.KIVIO_PERF_WEBKIT_MAX_THEME_MS, 20),
    },
  },
}

const baseUrl = config.baseUrl
const formulaBlock = String.raw`$$
\begin{aligned}
\nabla_\theta J(\theta)
&= \mathbb{E}_{\tau \sim p_\theta(\tau)}
\left[\sum_{t=0}^{T} \nabla_\theta \log \pi_\theta(a_t\mid s_t)\,G_t\right] \\
\frac{\partial}{\partial x}\left(\int_{0}^{\infty} e^{-xt}\sin(t^2)\,dt\right)
&= -\int_{0}^{\infty} t e^{-xt}\sin(t^2)\,dt
\end{aligned}
$$`

function formulaAnswer(count) {
  const blocks = Array.from({ length: count }, (_, index) => `公式 ${index + 1}：

${formulaBlock}`)

  return `下面是 ${count} 个公式：

${blocks.join('\n\n')}

生成已经结束。`
}

const plainAnswer = `下面是一段普通回复，没有公式。

生成已经结束。`

function conversation(testCase) {
  return {
    id: testCase.id,
    title: testCase.formulaCount > 0 ? `公式卡顿测试 x${testCase.formulaCount}` : '普通短对话',
    provider_id: 'dev-provider',
    model: 'dev-model',
    messages: [
      {
        id: `${testCase.id}-u1`,
        role: 'user',
        content: testCase.formulaCount > 0 ? `生成 ${testCase.formulaCount} 个公式` : '生成一段普通回复',
        attachments: [],
        artifacts: [],
        tool_calls: [],
        segments: [],
        timestamp: now - 20,
      },
      {
        id: `${testCase.id}-a1`,
        role: 'assistant',
        content: testCase.assistantContent,
        attachments: [],
        artifacts: [],
        tool_calls: [],
        segments: [],
        stream_outcome: 'completed',
        timestamp: now - 10,
      },
    ],
    active_skill_id: null,
    activeSkillId: null,
    assistant_id: null,
    assistantId: null,
    assistant_snapshot: null,
    assistantSnapshot: null,
    created_at: now - 30,
    updated_at: now - 5,
    pinned: false,
    project_id: null,
    projectId: null,
    set_id: null,
    setId: null,
    agent_todo_state: { items: [], updated_at: 0 },
    agentTodoState: { items: [], updated_at: 0 },
    agent_plan_state: { mode: 'act', status: 'empty', plan: null, updated_at: 0 },
    agentPlanState: { mode: 'act', status: 'empty', plan: null, updated_at: 0 },
  }
}

const cases = [
  { name: 'plain_short_chat', id: 'conv_plain_perf', formulaCount: 0, assistantContent: plainAnswer },
  ...config.formulaCounts.map((count) => ({
    name: `formula_shadow_katex_x${count}`,
    id: `conv_formula_perf_${count}`,
    formulaCount: count,
    assistantContent: formulaAnswer(count),
  })),
]
const conversations = cases.map(conversation)

function aggregate(values) {
  const sorted = [...values].sort((a, b) => a - b)
  const pick = (p) => sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * p))]
  return {
    avg: values.reduce((sum, value) => sum + value, 0) / values.length,
    p95: pick(0.95),
    max: sorted[sorted.length - 1],
  }
}

function roundStats(stats) {
  return Object.fromEntries(
    Object.entries(stats).map(([key, value]) => [key, Number(value.toFixed(2))]),
  )
}

function blockingMax(row, opName) {
  return row.opBlockingMaxAcrossRuns[opName]?.max ?? 0
}

function assertSmoke(rows) {
  const errors = []
  const rowFor = (browser, formulaCount) => rows.find((row) => row.browser === browser && row.formulaCount === formulaCount)

  for (const browser of config.browsers) {
    const plain = rowFor(browser, 0)
    if (!plain) {
      errors.push(`${browser}: missing plain row`)
      continue
    }
    for (const opName of ['sidebar_class_toggle', 'search_dialog_toggle', 'theme_color_toggle']) {
      if (!plain.opBlockingMaxAcrossRuns[opName]) {
        errors.push(`${browser}: missing plain ${opName} metric`)
      }
    }

    for (const count of config.formulaCounts) {
      const formula = rowFor(browser, count)
      if (!formula) {
        errors.push(`${browser}: missing formula row for count ${count}`)
        continue
      }
      if (formula.normalKatexNodes !== 0) {
        errors.push(`${browser} x${count}: expected formula normalKatexNodes=0, got ${formula.normalKatexNodes}`)
      }
      if (formula.shadowHostCount !== count) {
        errors.push(`${browser} x${count}: expected shadowHostCount=${count}, got ${formula.shadowHostCount}`)
      }
      if (formula.shadowKatexNodes < count) {
        errors.push(`${browser} x${count}: expected formula KaTeX nodes inside shadow DOM`)
      }
      for (const opName of ['sidebar_class_toggle', 'search_dialog_toggle', 'theme_color_toggle']) {
        if (!formula.opBlockingMaxAcrossRuns[opName]) {
          errors.push(`${browser} x${count}: missing ${opName} metric`)
        }
      }
    }
  }

  const webkitPlain = rowFor('webkit', 0)
  if (webkitPlain) {
    for (const count of config.formulaCounts) {
      const webkitFormula = rowFor('webkit', count)
      if (!webkitFormula) continue
      for (const [opName, maxMs] of Object.entries(config.thresholds.webkitMaxBlockingMsByOp)) {
        const plainMax = blockingMax(webkitPlain, opName)
        const formulaMax = blockingMax(webkitFormula, opName)
        const limit = Math.max(maxMs, plainMax * config.thresholds.webkitFormulaToPlainRatio)
        if (formulaMax > limit) {
          errors.push(`webkit x${count}: ${opName} max ${formulaMax}ms exceeds ${Number(limit.toFixed(2))}ms (plain ${plainMax}ms)`)
        }
      }
    }
  }

  if (errors.length > 0) {
    throw new Error(`KaTeX perf smoke failed:\n- ${errors.join('\n- ')}`)
  }
}

async function waitForChat(page) {
  await page.waitForSelector('.chat-window-shell', { timeout: 15000 })
  await page.waitForSelector('.chat-markdown', { timeout: 15000 })
  await page.waitForTimeout(250)
}

async function installData(page, targetId) {
  await page.goto(`${baseUrl}#chat/${targetId}`, { waitUntil: 'domcontentloaded' })
  await page.evaluate(({ storageKey, lastRouteKey, conversations, targetId }) => {
    localStorage.setItem(storageKey, JSON.stringify(conversations))
    localStorage.setItem(lastRouteKey, `#chat/${targetId}`)
  }, { storageKey, lastRouteKey, conversations, targetId })
  await page.goto(`${baseUrl}#chat/${targetId}`, { waitUntil: 'networkidle' })
  await waitForChat(page)
}

async function measureCase(page, testCase) {
  await installData(page, testCase.id)
  return page.evaluate(async ({ iterations }) => {
    const stats = (xs) => {
      const sorted = [...xs].sort((a, b) => a - b)
      const pick = (p) => sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * p))]
      return {
        avg: xs.reduce((sum, x) => sum + x, 0) / xs.length,
        p95: pick(0.95),
        max: sorted[sorted.length - 1],
      }
    }
    const sleepFrames = (n = 2) => new Promise((resolve) => {
      let left = n
      const step = () => {
        left -= 1
        if (left <= 0) resolve(undefined)
        else requestAnimationFrame(step)
      }
      requestAnimationFrame(step)
    })
    const root = document.querySelector('.chat-window-shell')
    const main = document.querySelector('.chat-main-pane')
    const sidebar = document.querySelector('.chat-sidebar-shell')
    const searchButton = [...document.querySelectorAll('.chat-sidebar-shell button, button')].find((button) => {
      const label = button.getAttribute('aria-label') ?? ''
      const title = button.getAttribute('title') ?? ''
      const text = button.textContent?.replace(/\s+/g, ' ').trim() ?? ''
      return label === '搜索对话' || label === '搜索' || title === '搜索对话' || title === '搜索' || text === '搜索'
    })
    const normalKatexNodes = document.querySelectorAll('.chat-markdown .katex *').length
    const shadowHosts = [...document.querySelectorAll('[data-katex-shadow-host="true"]')]
    const shadowKatexNodes = shadowHosts.reduce(
      (sum, host) => sum + (host.shadowRoot?.querySelectorAll('.katex *').length ?? 0),
      0,
    )
    const frameDeltas = []
    let prev = performance.now()
    let running = true
    requestAnimationFrame(function raf(t) {
      frameDeltas.push(t - prev)
      prev = t
      if (running) requestAnimationFrame(raf)
    })

    function forceLayout() {
      return (root?.offsetWidth ?? 0) + (main?.offsetWidth ?? 0) + document.body.offsetHeight
    }

    async function op(name, fn, iterations = 24) {
      const blockingTimes = []
      const settleTimes = []
      for (let i = 0; i < iterations; i += 1) {
        const start = performance.now()
        await fn(i)
        forceLayout()
        blockingTimes.push(performance.now() - start)
        await sleepFrames(2)
        settleTimes.push(performance.now() - start)
      }
      return { name, blocking: stats(blockingTimes), settle: stats(settleTimes) }
    }

    const ops = []
    if (sidebar) {
      ops.push(await op('sidebar_class_toggle', async (i) => {
        sidebar.classList.toggle('is-collapsed', i % 2 === 0)
      }, iterations.sidebar))
    }
    if (searchButton) {
      ops.push(await op('search_dialog_toggle', async () => {
        searchButton.click()
        await sleepFrames(1)
        const dialog = document.querySelector('[role="dialog"][aria-label="搜索对话"]')
        if (dialog) {
          document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
          window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
        }
      }, iterations.search))
    }
    ops.push(await op('theme_color_toggle', async (i) => {
      document.documentElement.dataset.themeColor = i % 2 === 0 ? 'warm' : 'neutral'
    }, iterations.theme))

    running = false
    await sleepFrames(1)
    return {
      frame: stats(frameDeltas.slice(2)),
      ops,
      normalKatexNodes,
      shadowKatexNodes,
      shadowHostCount: shadowHosts.length,
    }
  }, { iterations: config.iterations })
}

async function installTauriMock(page) {
  await page.addInitScript(({ conversations, now }) => {
    const getConversation = (id) => conversations.find((item) => item.id === id)
    const toListItem = (conversation) => {
      const preview = [...conversation.messages].reverse().find(
        (message) => message.role === 'assistant' || message.role === 'user',
      )?.content ?? ''
      return {
        id: conversation.id,
        title: conversation.title,
        preview: preview.length > 100 ? `${preview.slice(0, 100)}...` : preview,
        provider_id: conversation.provider_id,
        model: conversation.model,
        message_count: conversation.messages.length,
        created_at: conversation.created_at,
        updated_at: conversation.updated_at,
        pinned: conversation.pinned,
        project_id: conversation.project_id ?? null,
        projectId: conversation.projectId ?? null,
        set_id: conversation.set_id ?? null,
        setId: conversation.setId ?? null,
        assistant_id: conversation.assistant_id ?? null,
        assistantId: conversation.assistantId ?? null,
        assistant_name: null,
        assistantName: null,
      }
    }
    const contextState = () => ({
      estimated_input_tokens: 1200,
      context_window_tokens: 200000,
      context_window_estimated: true,
      usage_ratio: 0.006,
      status: 'normal',
      segments: [],
      last_measured_at: now,
      last_compressed_at: null,
      compressed_message_count: 0,
      summary: null,
    })
    const settings = {
      hotkey: 'Alt+Space',
      theme: 'light',
      themeColor: 'neutral',
      targetLang: 'zh',
      source: 'auto',
      autoPaste: false,
      launchAtStartup: false,
      translatorProviderId: 'dev-provider',
      translatorModel: 'dev-model',
      chatProviderId: 'dev-provider',
      chatModel: 'dev-model',
      defaultModels: {
        chat: { providerId: 'dev-provider', model: 'dev-model' },
        vision: { providerId: 'dev-provider', model: 'dev-model' },
        titleSummary: { providerId: 'dev-provider', model: 'dev-model' },
        compression: { providerId: 'dev-provider', model: 'dev-model' },
        imageGeneration: { providerId: 'dev-provider', model: 'dev-model' },
      },
      chat: {
        streamEnabled: true,
        thinkingEnabled: true,
        maxOutputTokens: 4096,
        defaultLanguage: 'zh',
        systemPrompt: '',
        userDisplayName: '',
        userAvatar: '',
        defaultAgentRuntime: { kind: 'builtin', externalAgentId: null, externalModel: null, externalReasoning: null },
      },
      chatMemory: { enabled: false, toolWriteConfirm: true },
      providers: [{
        id: 'dev-provider',
        name: 'Dev Provider',
        apiKeys: ['test'],
        baseUrl: 'http://localhost',
        availableModels: ['dev-model'],
        enabledModels: ['dev-model'],
        supportsTools: true,
        enabled: true,
        apiFormat: 'openai_chat',
        compressRequestBody: false,
        modelOverrides: {},
      }],
      chatTools: {
        enabled: false,
        servers: [],
        skillScanPaths: [],
        skillAutoMatch: true,
        skillFallbackMode: 'progressive',
        skillScriptAllowlist: ['python3', 'bash', 'sh', 'node'],
        disabledSkillIds: [],
        maxToolRounds: 20,
        toolTimeoutMs: 60000,
        mcpIdleTimeoutMs: 600000,
        maxToolOutputChars: null,
        approvalPolicy: 'readonly_auto_sensitive_confirm',
        subAgentConcurrency: 12,
        nativeTools: {
          webSearch: true,
          webFetch: true,
          skillRuntime: true,
          readFile: true,
          writeFile: true,
          editFile: true,
          runCommand: true,
          runPython: true,
          workspaceRoots: [],
        },
      },
      documentProcessing: { ocrEngine: 'off', pdfStrategy: 'text' },
      knowledgeBase: { hybridEnabled: true, weightVector: 0.7, weightKeyword: 0.3, rerankProviderId: '', rerankModel: '' },
      retryEnabled: true,
      retryAttempts: 2,
      screenshotTranslation: { enabled: false },
      lens: { enabled: false },
      settingsLanguage: 'zh',
      autoCheckUpdate: false,
      imageArchiveEnabled: false,
      imageArchivePath: '',
    }
    let callbackId = 1
    window.__TAURI_INTERNALS__ = {
      metadata: {
        currentWindow: { label: 'chat' },
        currentWebview: { label: 'chat' },
      },
      transformCallback: () => callbackId++,
      invoke: async (cmd, args = {}) => {
        if (cmd === 'get_settings') return settings
        if (cmd === 'save_settings') return args.settings ?? settings
        if (cmd === 'get_version' || cmd === 'plugin:app|version') return 'test'
        if (cmd === 'set_chat_window_background') return null
        if (cmd === 'chat_get_conversations') return { success: true, conversations: conversations.map(toListItem) }
        if (cmd === 'chat_search_conversations') return { success: true, conversations: conversations.map(toListItem) }
        if (cmd === 'chat_get_conversation') {
          const conversation = getConversation(args.conversationId)
          if (!conversation) return { success: false, error: 'not found' }
          const state = contextState()
          return { success: true, conversation: { ...conversation, context_state: state, contextState: state } }
        }
        if (cmd === 'chat_get_context_stats') {
          const conversation = getConversation(args.conversationId)
          if (!conversation) return { success: false, error: 'not found' }
          const state = contextState()
          return { success: true, contextState: state, conversation: { ...conversation, context_state: state, contextState: state } }
        }
        if (cmd === 'chat_get_projects') return { success: true, projects: [] }
        if (cmd === 'chat_get_sets') return { success: true, sets: [] }
        if (cmd === 'chat_get_assistants') return { success: true, assistants: [] }
        if (cmd === 'chat_mcp_list_tools') return { success: true, tools: [] }
        if (cmd === 'chat_skills_list') return { success: true, skills: [], warnings: [] }
        if (cmd === 'chat_take_external_sends') return { success: true, requests: [] }
        if (cmd === 'chat_detect_external_agents') return { success: true, agents: [] }
        if (cmd === 'chat_list_external_cli_slash_commands') return { success: true, supportsSlashCommands: false, commands: [], message: null }
        if (cmd === 'chat_list_background_commands') return []
        if (cmd === 'chat_kill_background_command') return null
        if (cmd === 'chat_memory_get') {
          return {
            success: true,
            l1: { layer: 'l1', content: '', bytes: 0, maxBytes: 12000 },
            l2: { layer: 'l2', content: '', bytes: 0, maxBytes: 12000 },
            dir: '',
          }
        }
        if (cmd === 'chat_reasoning_efforts_for_model') return []
        if (cmd === 'plugin:event|listen') return callbackId++
        if (cmd === 'plugin:event|unlisten') return null
        if (cmd === 'plugin:window|is_maximized') return false
        if (cmd === 'plugin:window|scale_factor') return 1
        if (cmd === 'plugin:window|inner_size') return { width: 1280, height: 800 }
        if (cmd.startsWith('plugin:window|')) return null
        return null
      },
    }
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: () => {},
    }
  }, { conversations, now })
}

async function run(browserName, browserType) {
  const browser = await browserType.launch({ headless: true })
  const page = await browser.newPage({ viewport: config.viewport, deviceScaleFactor: config.deviceScaleFactor })
  await installTauriMock(page)
  page.on('console', (msg) => {
    if (msg.type() === 'error') console.error(`[${browserName} console]`, msg.text())
  })
  const rows = []
  for (const testCase of cases) {
    const runs = []
    for (let i = 0; i < config.runs; i += 1) {
      runs.push(await measureCase(page, testCase))
    }
    const opNames = [...new Set(runs.flatMap((run) => run.ops.map((op) => op.name)))]
    rows.push({
      browser: browserName,
      case: testCase.name,
      formulaCount: testCase.formulaCount,
      runs: config.runs,
      iterations: config.iterations,
      normalKatexNodes: runs[0].normalKatexNodes,
      shadowKatexNodes: runs[0].shadowKatexNodes,
      shadowHostCount: runs[0].shadowHostCount,
      frameMaxAcrossRuns: roundStats(aggregate(runs.map((run) => run.frame.max))),
      opBlockingMaxAcrossRuns: Object.fromEntries(opNames.map((name) => [
        name,
        roundStats(aggregate(runs.map((run) => run.ops.find((op) => op.name === name)?.blocking.max ?? 0))),
      ])),
      opSettleMaxAcrossRuns: Object.fromEntries(opNames.map((name) => [
        name,
        roundStats(aggregate(runs.map((run) => run.ops.find((op) => op.name === name)?.settle.max ?? 0))),
      ])),
    })
  }
  await browser.close()
  return rows
}

const all = []
for (const name of config.browsers) {
  all.push(...await run(name, browserTypes[name]))
}

console.log(JSON.stringify({ config, rows: all }, null, 2))
assertSmoke(all)
