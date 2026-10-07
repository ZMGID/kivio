import { api, type ChatStreamPayload } from '../../src/api/tauri'
import { chatApi } from '../../src/chat/api'
import type { Conversation, PendingAttachment, StudyMessageSource } from '../../src/chat/types'
import { CHAT_PROTOCOL_VERSION } from '../../src/generated/chatProtocol'

// Test-only provider transport around the actual shared Chat API and storage.
// No Study request implementation, duplicate chat state owner, or live model call.
const readingReplies = {
  'amano-abstract': '演示阅读（非真实模型调用）：前两句可译为：英语作为科学界的通用语言，是非英语母语者充分贡献于科学的一大障碍；但很少有研究量化这些语言障碍对他们职业发展的影响。impediment 在这里是“阻碍、障碍”。原文主张：对 908 名环境科学研究者的调查发现，非英语母语者，尤其职业早期的研究者，在用英语开展科研活动时付出更多努力。背景解释：这里说的是这项调查中的负担，不能直接推广成所有人的固定差距。',
  'amano-figure': '演示读图（非真实模型调用）：原图信息：横轴是已发表英文论文数，图注说明采用 log10 变换；A、B 的纵轴是分钟，C、D 是天，E、F 是百分比。粉色、绿色、深蓝色分别表示英语母语、中等和较低熟练度；实线圆点代表高收入组，虚线三角代表中低收入组。阴影表示拟合关系的 95% 置信区间。解释：A 图呈现英语阅读时间、发表经验及语言组别之间的关系；这些关系本身不能证明因果，也不能从图片编造精确效应值或 p 值。',
  'amano-whole-paper': '演示范围说明（非真实模型调用）：我目前只收到第 1 页的摘要选区，不能据此总结整篇论文的全部方法、结果和局限。可先概括当前摘要；要讨论具体方法或局限，请打开相应页面或框选相关段落。',
  'amano-missing-legend': '演示缺失上下文（非真实模型调用）：当前选区只有 A 面板，缺少图例和完整图注，不能仅凭颜色判断组别。请扩大选区以包括图例、横轴说明和图注，再解释各条线与阴影的含义。',
} as const
export type FixtureLesson = 'mit-5b-13' | 'mit-5f-2a' | keyof typeof readingReplies
export type FixtureRequest = { kind: 'send' | 'retry'; conversationId: string; content: string; attachments: PendingAttachment[]; studySource?: StudyMessageSource; model: string; providerId: string }
export type SavedImage = { path: string; name: string; mimeType: string; imageDataUrl: string }
declare global { interface Window { __studyTest: {
  requests: FixtureRequest[]; savedImages: SavedImage[]; cancelled: string[]; packets: string[];
  readImage: (path: string) => Promise<string | null>; failNext: boolean; failAttachmentNext: boolean; hold: boolean; lesson?: FixtureLesson; rawReply?: string;
} } }
window.__studyTest = { readImage: readFixtureImage, requests: [], savedImages: [], cancelled: [], packets: [], failNext: false, failAttachmentNext: false, hold: false }
const wait = () => new Promise<void>(resolve => window.setTimeout(resolve, 100))
const listeners = new Set<(packet: ChatStreamPayload) => void>()
const active = new Map<string, { cancelled: boolean }>()
const key = 'kivio-chat-dev-conversations'

function saveConversation(conversation: Conversation) {
  const saved = JSON.parse(localStorage.getItem(key) ?? '[]') as Conversation[]
  const index = saved.findIndex(item => item.id === conversation.id)
  if (index < 0) throw new Error('The normal browser Chat API did not persist the conversation')
  saved[index] = conversation
  localStorage.setItem(key, JSON.stringify(saved))
}
async function imageDatabase() {
  return new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open('kivio-study-browser-attachments', 1)
    request.onupgradeneeded = () => request.result.createObjectStore('images', { keyPath: 'path' })
    request.onerror = () => reject(request.error)
    request.onsuccess = () => resolve(request.result)
  })
}
export async function readFixtureImage(path: string) {
  const db = await imageDatabase()
  return new Promise<string | null>((resolve, reject) => {
    const request = db.transaction('images', 'readonly').objectStore('images').get(path)
    request.onerror = () => { db.close(); reject(request.error) }
    request.onsuccess = () => { db.close(); resolve(request.result?.imageDataUrl ?? null) }
  })
}
function answer(source?: StudyMessageSource) {
  const { lesson, rawReply } = window.__studyTest
  if (rawReply) return rawReply
  if (source?.mode === 'read' && lesson && lesson in readingReplies) return readingReplies[lesson as keyof typeof readingReplies]
  if (lesson === 'mit-5b-13' && source?.mode === 'check') return '演示检查（非真实模型调用）：第一处问题在第三行，换元时漏掉了系数 $\\frac{1}{3}$。由 $du=3x^2\\,dx$，先把 $x^2\\,dx$ 改写成 $\\frac{1}{3}du$，再继续。'
  if (lesson === 'mit-5b-13' && source?.mode === 'hint') return '演示提示（非真实模型调用）：题目已经提示 $u=x^3$。下一步先写出 $du$ 与 $x^2\\,dx$ 的关系。'
  if (lesson === 'mit-5f-2a') return '演示检查（非真实模型调用）：第三行的加号应为减号。对照分部积分公式，自己修正第三行。'
  return 'Simulated shared Chat reply: explain the supplied source and distinguish it from your interpretation.'
}

async function simulateRun(conversation: Conversation, request: FixtureRequest) {
  const assistant = conversation.messages.pop()!
  const response = answer(request.studySource)
  const state = { cancelled: false }
  const fail = window.__studyTest.failNext
  window.__studyTest.failNext = false
  active.set(conversation.id, state)
  saveConversation(conversation) // Same canonical browser storage, with the committed user turn.
  const runId = `simulated-run-${crypto.randomUUID()}`
  let seq = 0
  const emit = (event: Pick<ChatStreamPayload, 'type'> & Record<string, unknown>) => {
    const packet = { protocolVersion: CHAT_PROTOCOL_VERSION, scope: 'run', conversationId: conversation.id,
      runId, messageId: assistant.id, seq: ++seq, baseRevision: conversation.revision, ...event } as ChatStreamPayload
    window.__studyTest.packets.push(packet.type)
    for (const listener of listeners) listener(packet)
  }
  emit({ type: 'run_started', recovery: null })
  await wait()
  if (fail) {
    active.delete(conversation.id)
    emit({ type: 'run_failed', error: 'Simulated provider unavailable. Try again.', full: '', conversationRevision: conversation.revision })
    throw Object.assign(new Error('Simulated provider unavailable. Try again.'), { conversation: structuredClone(conversation) })
  }
  const split = Math.max(1, Math.floor(response.length / 2))
  emit({ type: 'text_delta', delta: response.slice(0, split), segment: null })
  while (window.__studyTest.hold && !state.cancelled) await wait()
  await wait()
  const full = state.cancelled ? response.slice(0, split) : response
  if (!state.cancelled) emit({ type: 'text_delta', delta: response.slice(split), segment: null })
  conversation.messages.push({ ...assistant, content: full, stream_outcome: state.cancelled ? 'cancelled' : 'completed' })
  conversation.revision += 1
  saveConversation(conversation)
  emit({ type: state.cancelled ? 'run_cancelled' : 'run_completed', full, conversationRevision: conversation.revision })
  active.delete(conversation.id)
  return conversation
}

export function installChatFixture() {
  // Native subscriptions/snapshot IPC are external boundaries too. The browser
  // has no desktop window; only the normal run stream is supplied below.
  for (const name of Object.keys(api)) if (name.startsWith('on')) Reflect.set(api, name, async () => () => {})
  api.chatSyncState = async () => {}
  api.scheduledTasksList = async () => []
  api.onChatStream = async listener => { listeners.add(listener); return () => { listeners.delete(listener) } }
  api.chatSavePastedImage = async (name, mimeType, dataBase64) => {
    if (window.__studyTest.failAttachmentNext) { window.__studyTest.failAttachmentNext = false; return { success: false, error: 'Simulated attachment storage full' } }
    const saved: SavedImage = { path: `/simulated-attachments/${crypto.randomUUID()}.png`, name, mimeType, imageDataUrl: `data:${mimeType};base64,${dataBase64}` }
    const db = await imageDatabase()
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction('images', 'readwrite')
      tx.objectStore('images').put(saved)
      tx.oncomplete = () => { db.close(); resolve() }
      tx.onerror = () => { db.close(); reject(tx.error) }
    })
    window.__studyTest.savedImages.push(saved)
    return { success: true, path: saved.path, name, mimeType }
  }
  const send = chatApi.sendMessage.bind(chatApi)
  chatApi.sendMessage = async (...args) => {
    const [conversationId, content, attachments = [], , , , studySource] = args
    const before = await chatApi.getConversation(conversationId)
    const request: FixtureRequest = { kind: 'send', conversationId, content, attachments: structuredClone(attachments), studySource: structuredClone(studySource), model: before.model, providerId: before.provider_id }
    window.__studyTest.requests.push(request)
    return simulateRun(await send(...args), request)
  }
  const regenerate = chatApi.regenerateMessage.bind(chatApi)
  chatApi.regenerateMessage = async (...args) => {
    const [conversationId, messageId, newContent] = args
    const before = await chatApi.getConversation(conversationId)
    const index = before.messages.findIndex(message => message.id === messageId)
    const user = before.messages[index].role === 'user' ? before.messages[index] : before.messages[index - 1]
    const request: FixtureRequest = { kind: 'retry', conversationId, content: newContent ?? user.content, attachments: structuredClone(user.attachments ?? []), studySource: structuredClone(user.study_source), model: before.model, providerId: before.provider_id }
    window.__studyTest.requests.push(request)
    return simulateRun(await regenerate(...args), request)
  }
  chatApi.cancelStream = async id => { window.__studyTest.cancelled.push(id); const run = active.get(id); if (run) run.cancelled = true }
}
