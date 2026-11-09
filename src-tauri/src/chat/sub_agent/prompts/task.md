{{#if asyncEnabled}}Spawn `tasks[]` concurrently; non-blocking IDs return immediately.{{#if hasBlockingAgents}} BLOCKING agents return inline.{{/if}}{{else}}Run `tasks[]` synchronously.{{/if}}

{{#if asyncEnabled}}{{asyncGuidance}}{{/if}}

Most specific agent; prefer one investigator/editor.{{#if scoutAvailable}} Read-only research MUST use `scout` only when files unknown.{{/if}}{{#if defaultAgent}} Default `{{defaultAgent}}`: omit `agent`, NEVER specify it.{{else}} Specify enabled role.{{/if}} Specialist `agent`: tasks[] only, NEVER top-level.
Shared edits: one integration owner, files per task. Skip build/lint/tests/formatters mid-flight; run once afterward.
Children start blank.{{#if ircEnabled}} Parent IRC steers immediately.{{/if}} `context`: shared Goal/Contract/interfaces, no per-task repetition. `task`: self-contained Target(files/non-goals), Change(steps/APIs), Acceptance(observable).
`name`: CamelCase ≤32, auto if omitted; address by name. `solutionSpace`: given fix/design or open causes/designs; NEVER work volume/siblings/coordination.

Agents:
{{#if spawningDisabled}}Agent spawning is currently disabled.
{{else}}{{#if hasModelMentions}}`m<N>`: user-tagged model (`<model agent="m<N>" name="…"/>`), not specialist; spawn only when user names it.
{{/if}}{{agentList}}{{/if}}
