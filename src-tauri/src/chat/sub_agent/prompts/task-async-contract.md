Results auto-deliver; NEVER poll.{{#if mainAgent}} Work independently; if only results remain, end with a brief handoff; results automatically resume the conversation.{{/if}}{{#if waitTool}} {{#if mainAgent}}Use `wait` only if you need the result within this same turn.{{else}} Completely blocked? Call `wait` for your first settled job.{{/if}}{{/if}}

Process-local job IDs; results expire ~30s delivered/~5min unconsumed. Read output/transcripts: `agent://<id>` / `history://<id>`.{{#if ircEnabled}} Message: `write agent://<id>`.{{/if}}

`completed`: success; artifacts unverified.
