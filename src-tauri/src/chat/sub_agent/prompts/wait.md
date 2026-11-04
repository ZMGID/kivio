Wait only when blocked with nothing else to do.
Blocks on background subagent jobs you started; returns when a job settles or the wait times out.
Nothing you started running? Returns at once.
- Wait only on your own jobs.
- If a result arrives while other jobs are still running, act on it before waiting again.
Results auto-deliver. NEVER poll while work remains.
