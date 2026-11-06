Results auto-deliver; NEVER poll.{{#if mainAgent}} Do independent work if there is any. When the only remaining step depends on background results, end this response with a brief handoff; those results automatically resume the conversation.{{/if}}{{#if waitTool}} {{#if mainAgent}}Use `wait` only if you need the result within this same turn.{{else}} Completely blocked? Call `wait` to receive the first settled job you started.{{/if}}{{/if}}

Job IDs are process-local; delivered results expire shortly (~30s), unconsumed results within ~5min. Agent output/transcripts remain readable at `agent://<id>` / `history://<id>`.{{#if ircEnabled}} `write agent://<id>` messages a live agent.{{/if}}

`completed`: the subagent finished successfully; claimed artifacts are unverified.
