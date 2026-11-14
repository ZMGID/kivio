type ParsedCliVersion = {
  core: number[]
  pre: Array<number | string> | null
}

/** 抓 `x.y.z` 以及可选 `-rc.7`。 */
function parseCliVersion(value: string): ParsedCliVersion | null {
  const match = value.match(/v?(\d+(?:\.\d+){2,})(?:-([0-9A-Za-z.-]+))?/)
  if (!match) return null
  const core = match[1].split('.').map(Number)
  if (!core.every(Number.isSafeInteger)) return null
  const pre = match[2]
    ? match[2].replace(/[.-]+$/, '').split('.').map((ident) => (
      /^\d+$/.test(ident) ? Number(ident) : ident
    ))
    : null
  return { core, pre }
}

function compareCore(local: number[], latest: number[]): number {
  const length = Math.max(local.length, latest.length)
  for (let index = 0; index < length; index += 1) {
    const localPart = local[index] ?? 0
    const latestPart = latest[index] ?? 0
    if (localPart !== latestPart) return localPart < latestPart ? -1 : 1
  }
  return 0
}

function comparePreIdent(local: number | string, latest: number | string): number {
  const localNum = typeof local === 'number'
  const latestNum = typeof latest === 'number'
  if (localNum && latestNum) return local === latest ? 0 : local < latest ? -1 : 1
  if (localNum) return -1
  if (latestNum) return 1
  return local === latest ? 0 : String(local) < String(latest) ? -1 : 1
}

function comparePrerelease(
  local: Array<number | string> | null,
  latest: Array<number | string> | null,
): number {
  if (local == null) return latest == null ? 0 : 1
  if (latest == null) return -1
  const length = Math.min(local.length, latest.length)
  for (let index = 0; index < length; index += 1) {
    const order = comparePreIdent(local[index], latest[index])
    if (order !== 0) return order
  }
  return local.length - latest.length
}

/**
 * 本地版本是否落后于最新版（含 prerelease）。任一边解析不了返回 `null`，
 * 由调用方退回后端给的 `updateAvailable`。
 *
 * 后端也算一遍，这里再算是因为本地版本有两个来源：安装信息里的，和探测列表里的。
 * 前者缺失时后端只能回 false，而列表那份往往是有的。
 */
export function isCliVersionOutdated(localVersion: string, latestVersion: string): boolean | null {
  const local = parseCliVersion(localVersion)
  const latest = parseCliVersion(latestVersion)
  if (!local || !latest) return null
  const core = compareCore(local.core, latest.core)
  if (core !== 0) return core < 0
  return comparePrerelease(local.pre, latest.pre) < 0
}
