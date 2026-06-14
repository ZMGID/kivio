# Fix duplicate new chat creation

## Goal

Prevent repeated clicks on "新建聊天" from creating multiple empty chat records. The Chat UI should reuse the current empty regular conversation and guard against rapid duplicate create requests, while preserving the ability to create a fresh chat after the current conversation has content.

## What I already know

- `Sidebar.tsx` forwards "新建聊天" clicks to `Chat.tsx`.
- `Chat.tsx` `handleNewConversation` currently calls `chatApi.createConversation` every time.
- The Rust `chat_create_conversation` command always generates a new `conv_*` ID and saves an empty conversation.
- Sending a first message already creates a conversation when none exists, so this fix should not disrupt send flow.

## Requirements

- Re-clicking "新建聊天" while the current conversation is already an empty regular conversation should not create another stored conversation.
- Rapid repeated clicks should not race multiple `createConversation` requests.
- After a conversation has messages, "新建聊天" should still create a new empty conversation.
- Assistant-started conversations should keep their existing behavior and create a new assistant conversation.

## Acceptance Criteria

- [ ] Clicking "新建聊天" several times from an empty new chat leaves only one empty current conversation.
- [ ] `Cmd/Ctrl+N` follows the same behavior as the button.
- [ ] Sending a first message from a blank chat still works.
- [ ] TypeScript typecheck passes for the changed frontend code.

## Out of Scope

- Backend storage migration or deletion of existing duplicate empty conversations.
- Changing assistant conversation creation semantics.
- Adding a new backend API contract.

## Technical Notes

- Primary file: `src/chat/Chat.tsx`.
- Prefer a frontend guard over changing `chat_create_conversation`, because the backend command is a generic creation primitive used by multiple entry points.
