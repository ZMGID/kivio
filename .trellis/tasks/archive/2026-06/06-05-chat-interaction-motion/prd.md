# Optimize Chat Interaction Motion

## Goal

Improve the Chat main client's interaction feel with lightweight, restrained motion. The work should make state transitions feel continuous without adding dependencies, changing backend behavior, or increasing runtime footprint.

## What I already know

* The user wants the previously proposed AI client motion plan implemented.
* The target surface is the Chat main client under `src/chat/*`, not Lens or Settings.
* Existing motion is mostly Tailwind transitions, CSS keyframes, `animate-spin`, `animate-pulse`, and a custom empty-state typewriter.
* `package.json` has no animation dependency such as Framer Motion; the plan explicitly avoids adding one.
* Existing relevant components include `Chat.tsx`, `MessageList.tsx`, `MessageBubble.tsx`, `InputBar.tsx`, `Sidebar.tsx`, `ConversationList.tsx`, selectors, context menus, `ToolCallBlock.tsx`, `ReasoningBlock.tsx`, and `ChatAttachments.tsx`.

## Requirements

* Add shared Chat motion utilities in `src/index.css`:
  * fade-up entrance for messages, empty hero content, attachments, and transient UI.
  * popover entrance for model/skill/tool panels and context menus.
  * row entrance for sidebar project/conversation rows.
  * soft pulse for enabled send/run states.
  * reduced-motion fallback through `prefers-reduced-motion`.
* Improve empty conversation to active conversation feel without changing routing, data flow, or message persistence.
* Add lightweight entrance motion to user messages, assistant messages, streaming placeholders, and errors.
* Add smooth reveal behavior to `ReasoningBlock` and `ToolCallBlock` detail content.
* Add motion to input drag hints, attachment previews, tool panel, and send/stop button state.
* Add popover motion to model selector, skill selector, and chat/project/section context menus.
* Add sidebar search reveal and row entrance with delay capped to the first 12 rows.
* Do not add npm dependencies, backend commands, Tauri APIs, Rust types, or persistent fields.

## Acceptance Criteria

* [ ] Empty chat hero remains visually consistent, with existing typewriter and dot-grid behavior preserved.
* [ ] First sent user message and assistant streaming placeholder enter with a subtle fade-up instead of an abrupt hard cut.
* [ ] Normal messages, streaming messages, and error messages have consistent restrained entrance motion.
* [ ] Model selector, Skill selector, tool panel, and context menus open without flicker or position shifts.
* [ ] Dragging images over the composer shows an animated hint and attachment previews enter cleanly.
* [ ] Reasoning and tool details expand/collapse smoothly and keep existing accessibility semantics.
* [ ] Sidebar search reveal and row entrance do not resize or destabilize the list layout.
* [ ] Reduced-motion users see static or near-static transitions.

## Definition of Done

* `npm run lint` passes.
* `npm run typecheck` passes.
* Manual smoke testing covers the scenarios listed in the user plan.
* No unrelated files or existing untracked user files are modified.

## Out of Scope

* Adding Framer Motion or any other animation package.
* Redesigning Chat layout or visual palette.
* Changing Lens, Settings, backend APIs, persisted schemas, or message streaming semantics.
* Implementing full exit animations that require delayed unmounting.

## Technical Notes

* Keep motion CSS centralized in `src/index.css`.
* Prefer class-based animation utilities over component-level animation abstractions.
* Existing style convention uses Tailwind utility classes, single quotes, no semicolons, and 2-space indentation.
* Existing untracked files before this task: `.playwright-cli/`, `previews/`, `scripts/__pycache__/`, `tmp-tool-test.txt`.
