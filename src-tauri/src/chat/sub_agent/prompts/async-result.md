<system-notice>
{{#if multiple}}{{jobCount}} background jobs have completed. Resume your work using the results below.

{{else}}Background job {{jobs.[0].jobId}} has completed. Resume your work using the result below.
{{/if}}{{#each jobs}}{{#if @root.multiple}}── Job {{this.jobId}}{{#if this.label}} ({{this.label}}){{/if}} ──
{{/if}}{{this.result}}{{#if this.showAgentUrl}}

Agent: agent://{{this.agentUrlId}}, history://{{this.agentUrlId}}{{/if}}{{#if this.showHistoryUrl}}

Agent: history://{{this.agentUrlId}}{{/if}}{{#unless @last}}
{{/unless}}{{/each}}
</system-notice>
