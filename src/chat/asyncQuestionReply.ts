/**
 * Codex 0.156+ async-question reply envelope (`<send_user_message_question_reply>`), shared with
 * the Codex desktop and TUI so a question answered in Kivio is dismissed everywhere and replays as
 * an answer. Shape and id grammar from codex-rs `context-fragments/answered_question.rs` and
 * `tui/bottom_pane/async_questions/state.rs`: `JSON.stringify(["request_user_input_async",
 * itemId, index])`.
 */
const OPEN = '<send_user_message_question_reply>'
const CLOSE = '</send_user_message_question_reply>'
const CODEX_ASYNC_TOOL_PREFIX = 'codex-async-'
const MAX_ID_BYTES = 512

export interface AsyncQuestionAnswer {
  index: number
  question: string
  answer: string
}

/** Envelope for a Codex async card, or `null` when the tool is not one (other CLIs keep plain text). */
export function codexAsyncReplyEnvelope(toolId: string, answers: AsyncQuestionAnswer[]): string | null {
  if (!toolId.startsWith(CODEX_ASYNC_TOOL_PREFIX) || !answers.length) return null
  const itemId = toolId.slice(CODEX_ASYNC_TOOL_PREFIX.length)
  const replies = answers.map(({ index, question, answer }) => ({
    questionItemId: JSON.stringify(['request_user_input_async', itemId, index]),
    question: question.slice(0, MAX_ID_BYTES).replace(/[\r\n]/g, ' '),
    answer,
  }))
  if (replies.some((reply) => new TextEncoder().encode(reply.questionItemId).length > MAX_ID_BYTES)) return null
  return `${OPEN}${JSON.stringify(replies)}${CLOSE}`
}

/** Readable form of an envelope (Codex TUI `display_text`); `null` when `text` is not one. */
export function asyncReplyDisplayText(text: string): string | null {
  const trimmed = text.trim()
  if (!trimmed.startsWith(OPEN) || !trimmed.endsWith(CLOSE)) return null
  try {
    const parsed: unknown = JSON.parse(trimmed.slice(OPEN.length, -CLOSE.length))
    const replies = Array.isArray(parsed) ? parsed : [parsed]
    const lines = replies.flatMap((reply) => {
      if (!reply || typeof reply !== 'object') return []
      const { question, answer } = reply as { question?: unknown; answer?: unknown }
      if (typeof question !== 'string' || typeof answer !== 'string') return []
      return [`> ${question}\n\n${answer}`]
    })
    return lines.length ? lines.join('\n\n') : null
  } catch {
    return null
  }
}
