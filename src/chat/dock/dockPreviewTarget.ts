export type DockPreviewTarget = {
  /** 查看器根目录 + 相对路径。 */
  request: { workdir: string; path: string }
  /** 文件树里可定位的相对路径；文件在 workdir 之外时为 null。 */
  revealRel: string | null
}

function normalizePath(value: string): string {
  return value.replace(/\\/g, '/').replace(/^\/\/\?\//, '')
}

/**
 * 工具卡片点文件名 → dock 查看器预览的目标解析。
 *
 * - workdir 内的路径（相对，或以 workdir 为前缀的绝对路径）同时在文件树里定位；
 * - workdir 外的绝对路径（如写到桌面的文件）用其所在目录作查看器根，不定位；
 * - 没有 workdir 时相对路径无处可挂，返回 null。
 */
export function resolveDockPreviewTarget(rawPath: string, workdir: string): DockPreviewTarget | null {
  const target = normalizePath(rawPath.trim())
  if (!target) return null
  const wd = normalizePath(workdir)
  const isAbsolute = /^(?:[a-zA-Z]:)?\//.test(target)
  if (isAbsolute) {
    if (wd && target.toLowerCase().startsWith(`${wd.toLowerCase()}/`)) {
      const revealRel = target.slice(wd.length + 1)
      return { request: { workdir, path: revealRel }, revealRel }
    }
    const idx = target.lastIndexOf('/')
    if (idx <= 0) return null
    return { request: { workdir: target.slice(0, idx), path: target.slice(idx + 1) }, revealRel: null }
  }
  if (!workdir) return null
  const revealRel = target.replace(/^\.\//, '')
  return { request: { workdir, path: revealRel }, revealRel }
}
