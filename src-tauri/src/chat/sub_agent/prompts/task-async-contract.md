Results auto-deliver; NEVER poll.{{#if waitTool}} Completely blocked? Call `wait` to receive the first settled job you started.{{/if}}

Job IDs are process-local; delivered results expire shortly (~30s), unconsumed results within ~5min. Agent output/transcripts remain readable at `agent://<id>` / `history://<id>`.{{#if ircEnabled}} `write agent://<id>` messages a live agent.{{/if}}

`completed`: the subagent finished successfully; claimed artifacts are unverified.
