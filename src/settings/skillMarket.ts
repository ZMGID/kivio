// 技能市场数据层：浏览 / 搜索 / owner 消歧 ClawHub 目录（clawhub.ai）。列表与搜索接口
// 均返回 CORS `*`，故直接前端 fetch。下载/安装走 Rust（chat_skills_install_from_url），
// 这里只负责产出带 ownerHandle 的下载链。移植自 LiveAgent clawHub.ts，精简掉 detail 端点。

export type ClawHubSort = 'downloads' | 'stars' | 'installs' | 'updated' | 'newest'

export type ClawHubSkillCard = {
  slug: string
  displayName: string
  summary: string
  latestVersion: string | null
  downloads: number
  stars: number
  installsCurrent: number
  updatedAt: number | null
  ownerHandle: string | null
  webUrl: string | null
  downloadUrl: string | null
}

export type ClawHubListResponse = {
  items: ClawHubSkillCard[]
  nextCursor: string | null
}

const CLAWHUB_API_BASE = 'https://clawhub.ai'

export const CLAWHUB_SORT_OPTIONS: Array<{ value: ClawHubSort; labelZh: string; labelEn: string }> = [
  { value: 'downloads', labelZh: '下载最多', labelEn: 'Most downloaded' },
  { value: 'stars', labelZh: '星标最多', labelEn: 'Most starred' },
  { value: 'installs', labelZh: '安装最多', labelEn: 'Most installed' },
  { value: 'updated', labelZh: '最近更新', labelEn: 'Recently updated' },
  { value: 'newest', labelZh: '最新发布', labelEn: 'Newest' },
]

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {}
}

function asString(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value.trim() : null
}

function asNullableNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function buildClawHubWebUrl(ownerHandle: string | null, slug: string) {
  if (!ownerHandle) return null
  return `${CLAWHUB_API_BASE}/${encodeURIComponent(ownerHandle)}/${encodeURIComponent(slug)}`
}

export function buildClawHubDownloadUrl(slug: string, ownerHandle?: string | null) {
  const url = new URL('/api/v1/download', CLAWHUB_API_BASE)
  url.searchParams.set('slug', slug)
  url.searchParams.set('tag', 'latest')
  // ClawHub 对重名 slug 返回 409，必须带 ownerHandle 消歧。
  if (ownerHandle) {
    url.searchParams.set('ownerHandle', ownerHandle)
  }
  return url.toString()
}

export function normalizeClawHubSkillCard(raw: unknown): ClawHubSkillCard | null {
  const item = asRecord(raw)
  const slug = asString(item.slug)
  if (!slug) return null
  const stats = asRecord(item.stats)
  const latestVersion = asRecord(item.latestVersion)
  const tags = asRecord(item.tags)
  const owner = asRecord(item.owner)
  const ownerHandle = asString(item.ownerHandle) ?? asString(owner.handle)

  return {
    slug,
    displayName: asString(item.displayName) ?? slug,
    summary: asString(item.summary) ?? '',
    latestVersion: asString(latestVersion.version) ?? asString(tags.latest) ?? asString(item.version),
    downloads: asNullableNumber(item.downloads) ?? asNullableNumber(stats.downloads) ?? 0,
    stars: asNullableNumber(item.stars) ?? asNullableNumber(stats.stars) ?? 0,
    installsCurrent:
      asNullableNumber(item.installsCurrent) ??
      asNullableNumber(item.installs) ??
      asNullableNumber(stats.installsCurrent) ??
      asNullableNumber(stats.installs) ??
      0,
    updatedAt: asNullableNumber(item.updatedAt),
    ownerHandle,
    webUrl: asString(item.webUrl) ?? buildClawHubWebUrl(ownerHandle, slug),
    downloadUrl: asString(item.downloadUrl) ?? buildClawHubDownloadUrl(slug, ownerHandle),
  }
}

async function fetchClawHubJson(url: URL, fetchImpl?: typeof fetch): Promise<unknown> {
  const impl = fetchImpl ?? globalThis.fetch
  const response = await impl(url.toString(), { headers: { Accept: 'application/json' } })
  const body = await response.text()
  if (!response.ok) {
    throw new Error(`ClawHub request failed with HTTP ${response.status}${body ? `: ${body}` : ''}`)
  }
  return JSON.parse(body) as unknown
}

export async function listClawHubSkills(params: {
  sort: ClawHubSort
  cursor?: string | null
  limit?: number
  fetchImpl?: typeof fetch
}): Promise<ClawHubListResponse> {
  const url = new URL('/api/v1/skills', CLAWHUB_API_BASE)
  url.searchParams.set('limit', String(params.limit ?? 24))
  url.searchParams.set('sort', params.sort)
  url.searchParams.set('nonSuspiciousOnly', 'true')
  if (params.cursor) {
    url.searchParams.set('cursor', params.cursor)
  }
  const json = asRecord(await fetchClawHubJson(url, params.fetchImpl))
  const items = Array.isArray(json.items)
    ? (json.items.map(normalizeClawHubSkillCard).filter(Boolean) as ClawHubSkillCard[])
    : []
  return { items, nextCursor: asString(json.nextCursor) }
}

export async function searchClawHubSkills(params: {
  query: string
  limit?: number
  fetchImpl?: typeof fetch
}): Promise<ClawHubSkillCard[]> {
  const url = new URL('/api/v1/search', CLAWHUB_API_BASE)
  url.searchParams.set('q', params.query)
  url.searchParams.set('limit', String(params.limit ?? 24))
  url.searchParams.set('nonSuspiciousOnly', 'true')
  const json = asRecord(await fetchClawHubJson(url, params.fetchImpl))
  return Array.isArray(json.results)
    ? (json.results.map(normalizeClawHubSkillCard).filter(Boolean) as ClawHubSkillCard[])
    : []
}

function narrow(candidates: ClawHubSkillCard[], predicate: (c: ClawHubSkillCard) => boolean) {
  const narrowed = candidates.filter(predicate)
  return narrowed.length > 0 ? narrowed : candidates
}

// 列表卡片常缺 ownerHandle，而重名 slug 有多个发布者。用 search 结果逐步按
// updatedAt/version/downloads/summary/displayName 收窄，收敛到唯一 owner 才返回。
export function selectClawHubOwnerCandidate(
  skill: ClawHubSkillCard,
  candidates: ClawHubSkillCard[],
): ClawHubSkillCard | null {
  let exact = candidates.filter(
    (c) => c.slug.toLowerCase() === skill.slug.toLowerCase() && Boolean(c.ownerHandle),
  )
  if (exact.length === 1) return exact[0]
  if (exact.length === 0) return null

  if (skill.updatedAt !== null) {
    exact = narrow(exact, (c) => c.updatedAt === skill.updatedAt)
    if (exact.length === 1) return exact[0]
  }
  if (skill.latestVersion) {
    exact = narrow(exact, (c) => c.latestVersion === skill.latestVersion)
    if (exact.length === 1) return exact[0]
  }
  if (skill.downloads > 0) {
    exact = narrow(exact, (c) => c.downloads === skill.downloads)
    if (exact.length === 1) return exact[0]
  }
  if (skill.summary) {
    exact = narrow(exact, (c) => c.summary === skill.summary)
    if (exact.length === 1) return exact[0]
  }
  if (skill.displayName) {
    exact = narrow(exact, (c) => c.displayName === skill.displayName)
  }
  return exact.length === 1 ? exact[0] : null
}

export async function resolveClawHubSkillOwner(
  skill: ClawHubSkillCard,
  fetchImpl?: typeof fetch,
): Promise<ClawHubSkillCard> {
  if (skill.ownerHandle) return skill

  const candidates = await searchClawHubSkills({ query: skill.slug, limit: 50, fetchImpl })
  const resolved = selectClawHubOwnerCandidate(skill, candidates)
  if (!resolved?.ownerHandle) {
    throw new Error(
      `技能 "${skill.slug}" 有多个发布者且无法自动确定，请到 ClawHub 主页手动选择。`,
    )
  }
  return {
    ...skill,
    ownerHandle: resolved.ownerHandle,
    webUrl: resolved.webUrl ?? buildClawHubWebUrl(resolved.ownerHandle, skill.slug),
    downloadUrl: buildClawHubDownloadUrl(skill.slug, resolved.ownerHandle),
  }
}
