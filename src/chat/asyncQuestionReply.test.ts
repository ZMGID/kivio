import { describe, expect, it } from 'vitest'
import { asyncReplyDisplayText, codexAsyncReplyEnvelope } from './asyncQuestionReply'

describe('Codex async question reply envelope', () => {
  it('uses the desktop question id grammar so other Codex clients dismiss the card', () => {
    const text = codexAsyncReplyEnvelope('codex-async-msg_1', [
      { index: 1, question: 'Which env?\nPick one', answer: 'Staging' },
    ])
    expect(text).toBe(
      '<send_user_message_question_reply>[{"questionItemId":"[\\"request_user_input_async\\",\\"msg_1\\",1]","question":"Which env? Pick one","answer":"Staging"}]</send_user_message_question_reply>',
    )
    expect(asyncReplyDisplayText(text!)).toBe('> Which env? Pick one\n\nStaging')
  })

  it('leaves non-Codex cards and ordinary text alone', () => {
    expect(codexAsyncReplyEnvelope('tool-1', [{ index: 0, question: 'q', answer: 'a' }])).toBeNull()
    expect(asyncReplyDisplayText('hello')).toBeNull()
    expect(asyncReplyDisplayText('<send_user_message_question_reply>nope</send_user_message_question_reply>')).toBeNull()
  })
})
