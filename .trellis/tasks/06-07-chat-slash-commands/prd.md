# brainstorm: Chat Slash Commands

## Goal

Add a foundational `/` command surface to the Kivio chat composer so users can discover and trigger safe local actions first, with a clean path to assistant workflows, skills, and session actions later.

## What I Already Know

- The user wants research first, comparing current Kivio with popular agent products such as Hermes Agent, Claude Code, and OpenCode.
- Kivio already has assistant quick command data in `ChatAssistant.quick_commands`, but these are intentionally deferred from the first foundation pass.
- Built-in assistants already define commands like `/整理`, `/改清楚`, `/下一步`, `/翻译`, `/润色`, and `/语气`; these are future product commands, not first-pass foundation commands.
- `InputBar` owns composer state, attachments, paste/drop handling, submit/cancel behavior, and the MCP/Skill settings popover.
- `Chat.tsx` already computes `currentAssistantSnapshot`, `enabledSkills`, `effectiveSkillId`, and sends `activeSkillId` through `chatApi.sendMessage`.
- `ChatAssistantSnapshot` includes `quick_commands`, so current-assistant commands can be surfaced without a backend schema change.

## Research References

- `research/agent-slash-command-patterns.md` — External command patterns and Kivio implementation recommendation.

## Requirements

- Typing `/` in the chat composer should open a command picker.
- The first pass should ship a small set of safe local commands so the foundation is testable without wiring assistant or skill behavior.
- The command list should show enough context to distinguish command name, action, and description.
- Selecting a command should be keyboard-friendly and should not break IME Enter behavior.
- Local commands should not send a model turn or mutate chat history unless explicitly designed to do so in a later phase.
- Destructive/session-mutating commands must require confirmation or be deferred.

## UI Direction

- Match the provided screenshot direction: a rounded command panel positioned directly above the composer, visually paired with the input box.
- Panel styling:
  - white/light surface with subtle border and soft shadow
  - large rounded corners similar to the composer shell
  - compact vertical list, no card nesting
  - selected row highlighted with a soft neutral gray rounded background
- Row layout:
  - left icon, command name, muted description on the same line
  - command name uses stronger weight/color
  - description uses lighter gray
  - row height stays stable while filtering
- The input itself should continue showing the typed `/` while the panel is open.
- The panel should feel like a command palette, not a tooltip or settings menu.

## Recommended Foundation MVP Scope

- Implement a generic slash command registry type and matching/filtering utilities.
- Add a command picker inside `InputBar`, visually aligned with the existing composer popover style.
- Support keyboard and mouse interaction for command discovery:
  - `/` opens the picker when typed at the start of the composer or after whitespace.
  - Typing after `/` filters commands.
  - Arrow keys move selection.
  - Enter applies the highlighted command.
  - Escape closes the picker.
  - IME composition must not trigger command apply/send.
- Support a minimal command execution contract that can represent:
  - local UI commands
  - prompt template commands
  - one-turn skill selection
  - future backend-dispatched commands
- Add only non-domain placeholder/test commands during development if needed, hidden behind local data or easy to remove.
- Do not wire assistant quick commands, skills, or destructive session commands into user-facing behavior in the foundation pass.

## First User-Facing Commands

- `/help` — opens or focuses the command picker/help list; local only, no model turn.
- `/settings` — opens the existing chat settings view; local only.
- `/tools` — opens the existing MCP / Skill tools surface or tool settings route; local only.
- `/attach` — opens the existing attachment picker; local only.

These commands exist to validate the command surface: discovery, filtering, selection, local action dispatch, and UI states. They should not alter conversation history.

## Out Of Scope For MVP

- Assistant quick command behavior such as `/整理`, `/翻译`, or `/润色`.
- User-facing Skill command invocation.
- Filesystem command discovery such as `.kivio/commands/*.md`.
- Shell-output interpolation like OpenCode `!command`.
- File-reference expansion like `@file`.
- Destructive commands such as `/clear`, `/delete`, `/reset`, or `/undo`.
- Cross-surface slash commands for external messaging platforms.

## Acceptance Criteria

- [ ] Typing `/` at the start of the composer can show a generic command picker when command definitions are provided.
- [ ] The command picker visually matches the provided screenshot direction: rounded panel above composer, icon/name/description rows, and soft gray selected row.
- [ ] The command picker can render command icon, slash/name, description, category/source label, and optional argument hint.
- [ ] Arrow keys, Enter, Escape, mouse click, and IME composition behave correctly.
- [ ] Filtering supports English and Chinese command names.
- [ ] Selecting a command calls a generic handler rather than hard-coded assistant behavior.
- [ ] `/help`, `/settings`, `/tools`, and `/attach` are wired as safe local commands.
- [ ] Running these commands does not create chat messages or call the model.
- [ ] The command picker can be mounted in both inline empty-chat layout and footer layout.
- [ ] The picker works in both inline empty-chat layout and footer layout.
- [ ] Existing attachment paste/drop and Enter-to-send behavior remain intact.

## Technical Notes

- Likely frontend files:
  - `src/chat/InputBar.tsx`
  - `src/chat/Chat.tsx`
  - `src/chat/types.ts`
- Likely backend files only if backend execution is added later:
  - `src-tauri/src/chat/types.rs`
  - `src-tauri/src/chat/storage.rs`
  - `src-tauri/src/chat/commands.rs`
- Important local constraint: recent IME send handling in `InputBar` must be preserved.
- Priority order should be: built-in exact, current assistant exact, enabled skill exact, other/global fuzzy matches.

## Deferred Product Questions

- Should a selected assistant quick command send immediately, or should it insert a prepared prompt and let the user press send?

Recommended answer: insert/prepare first for MVP. It gives users control, avoids surprise sends, and works better for commands that need arguments.
