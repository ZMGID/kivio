type PlanTone = 'free' | 'entry' | 'standard' | 'advanced' | 'premium' | 'flagship' | 'team' | 'enterprise'

function planTones(groups: ReadonlyArray<readonly [PlanTone, readonly string[]]>) {
  const tones = new Map<string, PlanTone>()
  for (const [tone, plans] of groups) {
    for (const plan of plans) tones.set(plan, tone)
  }
  return tones
}

// Match within each CLI because names such as Pro, Plus and Ultra have
// different meanings across subscriptions. Keep legacy display names valid.
const CLI_PLAN_TONES = new Map([
  ['codex', planTones([
    ['free', ['free']],
    ['entry', ['go']],
    ['standard', ['plus']],
    ['advanced', ['pro', 'pro 100', 'prolite']],
    ['premium', ['pro 20', 'pro 20x', 'pro 200']],
    ['flagship', ['pro 500', 'promax']],
    ['team', ['team', 'business', 'business standard', 'business premium']],
    ['enterprise', ['enterprise', 'edu']],
  ])],
  ['grok', planTones([
    ['free', ['free']],
    ['entry', ['plus', 'supergrok lite', 'x plus']],
    ['standard', ['premium', 'supergrok', 'x premium']],
    ['premium', ['super', 'supergrok plus', 'x super', 'x premium+', 'x premium plus']],
    ['flagship', ['ultra', 'supergrok heavy', 'x ultra']],
    ['team', ['team', 'teams', 'business']],
    ['enterprise', ['enterprise']],
  ])],
  ['antigravity', planTones([
    ['free', ['free', 'google ai free']],
    ['standard', ['google ai plus', 'plus']],
    ['advanced', ['google ai pro', 'pro']],
    ['premium', ['google ai ultra', 'google ai ultra 5x', 'ultra', 'ultra 5x']],
    ['flagship', ['google ai ultra 20x', 'ultra 20x']],
    ['enterprise', ['enterprise', 'gemini enterprise', 'gemini enterprise standard', 'gemini enterprise plus']],
  ])],
  ['claude', planTones([
    ['free', ['free']],
    ['standard', ['pro']],
    ['premium', ['max', 'max 5x']],
    ['flagship', ['ultra', 'max 20x']],
    ['team', ['team', 'team standard', 'team premium']],
    ['enterprise', ['enterprise', 'enterprise standard', 'enterprise premium']],
  ])],
  ['kimi', planTones([
    ['free', ['free']],
    ['entry', ['go', 'andante']],
    ['standard', ['plus', 'moderato']],
    ['advanced', ['pro', 'allegretto']],
    ['premium', ['max', 'allegro']],
    ['flagship', ['ultra']],
  ])],
])

export function cliPlanTone(agentId: string, plan: string) {
  const key = plan.toLowerCase().replace(/×/g, 'x').replace(/[\s_-]+/g, ' ').trim()
  return CLI_PLAN_TONES.get(agentId)?.get(key)
}
