// Pure helpers for the InputBar slash-command popover. Kept framework-free so
// the merge/match logic is unit-testable independently of React.
//
// Three kinds of slash commands coexist in the popover, distinguished by `kind`:
// - 'action' — built-in session actions (/help, /plan, /new ...) dispatched
//   locally on confirmation (onNewChat etc.), never sent as a message.
// - 'skill'  — user skills surfaced as `/name`; selecting one only completes the
//   token (`/name `), and the whole string is sent on Enter. The backend parses
//   the slash trigger and pins the skill (see try_apply_skill_slash_trigger).
// - 'cli'    — external agent CLI slash commands; completes the token and sends
//   on Enter (passthrough to the local CLI runtime).

export type SlashCommandKind = 'action' | 'skill' | 'cli'

export interface SlashCommandDefinition {
  id: string
  slash: `/${string}`
  title: string
  description: string
  category: string
  keywords: string[]
  kind: SlashCommandKind
  argumentHint?: string
  agentId?: string
}

export type ComposerToken = { start: number; end: number; query: string; trigger: '/' | '$' | '@' }

// Whitespace and CJK text may introduce a command. URL/path separators may not.
function isComposerBoundary(value: string, start: number): boolean {
  return start === 0 || /[\s\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}，。！？；：（）【】]/u.test(value[start - 1])
}

/** Keep a file path literal when a space or inline command could split the mention. */
export function formatComposerFileMention(path: string): string {
  const needsQuotes = /[\s`]/u.test(path) || Array.from(path.matchAll(/\//g)).some(match => isComposerBoundary(path, match.index))
  if (!needsQuotes) return `@${path}`
  const longestRun = (path.match(/`+/g) ?? []).reduce((length, run) => Math.max(length, run.length), 0)
  const fence = '`'.repeat(longestRun + 1)
  const pad = /^[`\s]|[`\s]$/u.test(path) ? ' ' : ''
  return `@${fence}${pad}${path}${pad}${fence}`
}

function composerTokens(value: string): ComposerToken[] {
  const tokens: ComposerToken[] = []
  let codeFence = ''
  for (let i = 0; i < value.length; i++) {
    if (value[i] === '`') {
      let end = i + 1
      while (value[end] === '`') end++
      const fence = value.slice(i, end)
      if (!codeFence) codeFence = fence
      else if (fence === codeFence) codeFence = ''
      i = end - 1
      continue
    }
    const trigger = value[i]
    if (codeFence || (trigger !== '/' && trigger !== '$' && trigger !== '@') || !isComposerBoundary(value, i)) continue
    if (value.slice(value.lastIndexOf('\n', i) + 1, i).trimStart().startsWith('>')) continue
    if (trigger === '$' && /[0-9]/.test(value[i + 1] ?? '')) continue
    let end = i + 1
    if (trigger === '@') {
      while (end < value.length && /[^\s`，。！？；：（）【】]/u.test(value[end])) end++
    } else {
      while (end < value.length && /[a-zA-Z0-9_:-]/.test(value[end])) end++
    }
    if (value[end] === '/' || value[end] === '.' || value[end] === '\\') continue
    tokens.push({ trigger, start: i, end, query: value.slice(i + 1, end) })
    i = end - 1
  }
  return tokens
}

export function findActiveComposerToken(value: string, cursor: number): ComposerToken | null {
  if (cursor < 0 || cursor > value.length) return null
  const token = composerTokens(value).find(token => token.start < cursor && cursor <= token.end)
  return token ? { ...token, query: value.slice(token.start + 1, cursor) } : null
}

export function findComposerCommands<T extends { slash: string }>(value: string, commands: readonly T[]) {
  return composerTokens(value).flatMap(token => {
    if (token.trigger !== '/') return []
    const command = commands.find(item => item.slash.toLowerCase() === `/${token.query.toLowerCase()}`)
    return command ? [{ start: token.start, end: token.end, command }] : []
  })
}

/** Minimal skill shape the popover needs. */
export interface SlashSkill {
  id: string
  name: string
  description?: string
  argumentHint?: string | null
  disableModelInvocation?: boolean
}

const SLASH_NAME_RE = /[^a-z0-9]+/g

/** Default `/slug(name)` trigger token for a skill (lowercased, dashed). */
export function skillSlashToken(skill: SlashSkill): `/${string}` {
  const slug = skill.name
    .toLowerCase()
    .replace(SLASH_NAME_RE, '-')
    .replace(/^-+|-+$/g, '')
  const token = slug || skill.id.toLowerCase()
  return `/${token}` as `/${string}`
}

/**
 * Merge built-in action commands with enabled skills into one popover list.
 * `disableModelInvocation` is NOT a filter here — it only gates model
 * auto-invocation, while an explicit user slash command may still trigger such
 * a skill. Callers pass already-enabled skills.
 */
export function buildSlashCommands(
  actions: SlashCommandDefinition[],
  enabledSkills: SlashSkill[],
): SlashCommandDefinition[] {
  const skillCommands = enabledSkills.map<SlashCommandDefinition>((skill) => {
    const slash = skillSlashToken(skill)
    return {
      id: `skill:${skill.id}`,
      slash,
      title: slash,
      description: skill.description?.trim() || skill.name,
      category: 'Skills',
      keywords: [skill.id, skill.name, slash.slice(1)].map((item) => item.toLowerCase()),
      kind: 'skill',
      argumentHint: skill.argumentHint?.trim() || undefined,
    }
  })
  return [...actions, ...skillCommands]
}

/** Case-insensitive fuzzy match against a slash command's searchable fields. */
export function commandMatches(command: SlashCommandDefinition, query: string): boolean {
  const normalized = query.trim().toLowerCase()
  if (!normalized) return true
  const searchable = [
    command.slash.slice(1),
    command.title,
    command.description,
    command.category,
    ...command.keywords,
  ].map((item) => item.toLowerCase())
  return searchable.some((item) => item.includes(normalized))
}
