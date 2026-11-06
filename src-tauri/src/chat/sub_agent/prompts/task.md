{{#if asyncEnabled}}Spawn `tasks[]` concurrently; non-blocking IDs return immediately.{{#if hasBlockingAgents}} BLOCKING agents return inline.{{/if}}{{else}}Run `tasks[]` synchronously.{{/if}}

{{#if asyncEnabled}}{{asyncGuidance}}{{/if}}

# Delegation
Use most specific agent.{{#if scoutAvailable}} Read-only research MUST use `scout` only when files unknown.{{/if}} Prefer one agent to investigate + edit.{{#if defaultAgent}} Omit `agent` only for default (`{{defaultAgent}}`); NEVER specify it.{{else}} Specify an enabled role explicitly.{{/if}}
Specialist `agent` selectors belong inside each `tasks[]` item; NEVER put `agent` at the top level.
Shared edits need one integration owner; give each task its files. Set interfaces in `context`. Every task MUST skip build/lint/tests/formatters mid-flight; run once afterward.

# Inputs
`name`: CamelCase ≤32, auto-generated if omitted; address agent by name.
`solutionSpace`: describe how open-ended the child's problem is: whether the fix or design is given, or which causes or designs remain open. Volume of work does not widen it; NEVER mention sibling agents or coordination. (`one fix: rename, names given`; `one fix: slice end in paginate`; `single-flight cache load; races easy to miss`; `several retry API shapes; error classes to choose`; `deadlock cause open, no repro`)

Children start blank;{{#if ircEnabled}} parent IRC steers immediately;{{/if}} pass shared background in `context`.

# Format
`context`: shared (`# Goal`, `# Contract` interfaces); NEVER repeat per task.
`task`: self-contained (`# Target` files/non-goals, `# Change` steps/APIs, `# Acceptance` observable result).

# Available Agents
{{#if spawningDisabled}}Agent spawning is currently disabled.
{{else}}{{#if hasModelMentions}}`m<N>` = user-tagged model (`<model agent="m<N>" name="…"/>`), not specialist; spawn only when user names it.
{{/if}}{{agentList}}{{/if}}
