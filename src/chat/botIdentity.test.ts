// @vitest-environment jsdom
import { expect, it } from 'vitest'
import {
  botAssistantId,
  botContactId,
  isBotOwnedConversation,
  isDedicatedMemberConversation,
  memberThreadIdentity,
} from './botIdentity'
import { belongsToWorkspace } from './workspace'

const memberId = 'conv_bot_member_assistant-1@room-9'
const contactId = 'conv_bot_assistant-1'

it('keeps a group execution thread out of the contact id', () => {
  expect(isDedicatedMemberConversation(memberId)).toBe(true)
  expect(botAssistantId(memberId)).toBeNull()
  expect(memberThreadIdentity(memberId)).toEqual({ assistantId: 'assistant-1', roomId: 'room-9' })
  expect(botContactId(memberId)).toBe('assistant-1')
  expect(botAssistantId(contactId)).toBe('assistant-1')
  expect(isBotOwnedConversation(memberId)).toBe(true)
  expect(isBotOwnedConversation('conv_agent')).toBe(false)
})

it('keeps member threads in the Bot workspace without treating them as Agent chats', () => {
  window.history.replaceState(null, '', '?workspace=bot')
  expect(belongsToWorkspace(memberId)).toBe(true)
  expect(belongsToWorkspace('conv_agent')).toBe(false)
  window.history.replaceState(null, '', '?workspace=')
  expect(belongsToWorkspace(memberId)).toBe(false)
  expect(belongsToWorkspace(contactId)).toBe(false)
})
