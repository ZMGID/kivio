# Add Default Model Settings

## Goal

Add a settings section named "默认模型" / "Default Models" with three optional model slots:

- Chat model: the global default model for new Chat conversations.
- Title summary model: the model intended for conversation title summarization.
- Compression model: the model intended for context/history compression.

Each slot can be configured independently or left unset.

## What I Already Know

- Chat currently stores its default as top-level `chatProviderId` / `chatModel` in frontend settings and `chat_provider_id` / `chat_model` in Rust.
- Chat creation and Chat UI both read those fields when deciding the initial provider/model.
- Existing `ModelPairSelect` already supports an optional empty/inherit entry for model choices.
- Context compression is not implemented yet. Conversation titles were previously generated locally by truncating the first user message.

## Requirements

- Add a structured settings block for default models.
- Preserve backward compatibility with existing `chatProviderId` / `chatModel` settings by migrating them into the chat default slot.
- Keep the legacy top-level fields available for older frontend/backend code paths during this transition.
- Allow all three slots to be unset in the UI.
- Use the configured chat default model for new Chat conversations when present.
- If the chat model slot is unset, keep the current fallback chain: Lens provider/model if configured, otherwise translator provider/model.
- Use the configured title summary model to generate the title for a new conversation after the first assistant reply; if it is unset, inherit the effective chat default model.
- If title summary generation fails, times out, or returns an invalid title, keep the existing local truncation fallback so chat sending still succeeds.
- If the compression slot is unset, treat it as inheriting the effective chat default model for future call sites.
- If a provider/model is deleted or disabled, clear or repair affected default-model slots consistently with existing settings behavior.

## Acceptance Criteria

- [ ] Settings UI shows a "默认模型" / "Default Models" group with Chat, title summary, and compression model selectors.
- [ ] Each selector offers an unset option.
- [ ] Saved settings include the new structured default model fields.
- [ ] Existing saved `chatProviderId` / `chatModel` values migrate into the new chat default slot.
- [ ] New Chat conversations use the new chat default slot when it is set.
- [ ] New Chat conversation titles use the title summary model after the first assistant reply, with local truncation as fallback.
- [ ] TypeScript typecheck and Rust tests cover the new settings contract.

## Out of Scope

- Implementing context/history compression.
- Retitling existing conversations retroactively.
- Redesigning provider management.

## Technical Notes

- Relevant files:
  - `src-tauri/src/settings.rs`
  - `src-tauri/src/chat/commands.rs`
  - `src/api/tauri.ts`
  - `src/settings/SettingsShell.tsx`
  - `src/settings/i18n.ts`
  - `src/chat/Chat.tsx`
- Relevant Trellis specs:
  - `.trellis/spec/frontend/type-safety.md`
  - `.trellis/spec/guides/code-reuse-thinking-guide.md`
