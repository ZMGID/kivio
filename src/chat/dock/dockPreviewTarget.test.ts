import { describe, expect, it } from 'vitest'
import { resolveDockPreviewTarget } from './dockPreviewTarget'

describe('resolveDockPreviewTarget', () => {
  const workdir = 'C:\\work\\proj'

  it('locates relative paths inside the workdir and strips a leading ./', () => {
    expect(resolveDockPreviewTarget('./src/a.ts', workdir)).toEqual({
      request: { workdir, path: 'src/a.ts' },
      revealRel: 'src/a.ts',
    })
  })

  it('treats absolute paths under the workdir as in-tree (case-insensitive, slash-agnostic)', () => {
    expect(resolveDockPreviewTarget('c:/WORK/proj/src/b.ts', workdir)).toEqual({
      request: { workdir, path: 'src/b.ts' },
      revealRel: 'src/b.ts',
    })
  })

  it('uses the parent directory as viewer root for absolute paths outside the workdir', () => {
    expect(resolveDockPreviewTarget('C:\\Users\\me\\Desktop\\out.md', workdir)).toEqual({
      request: { workdir: 'C:/Users/me/Desktop', path: 'out.md' },
      revealRel: null,
    })
  })

  it('handles posix absolute paths and the Windows long-path prefix', () => {
    expect(resolveDockPreviewTarget('/home/me/x.txt', '')).toEqual({
      request: { workdir: '/home/me', path: 'x.txt' },
      revealRel: null,
    })
    expect(resolveDockPreviewTarget('\\\\?\\C:\\work\\proj\\y.txt', workdir)?.revealRel).toBe('y.txt')
  })

  it('keeps the filesystem root for files directly under a drive or POSIX root', () => {
    expect(resolveDockPreviewTarget('/x.txt', workdir)?.request).toEqual({ workdir: '/', path: 'x.txt' })
    expect(resolveDockPreviewTarget('D:/x.txt', workdir)?.request).toEqual({ workdir: 'D:/', path: 'x.txt' })
    expect(resolveDockPreviewTarget('C:/x.txt', 'C:/')?.revealRel).toBe('x.txt')
  })

  it('returns null for blank input or a relative path without workdir', () => {
    expect(resolveDockPreviewTarget('   ', workdir)).toBeNull()
    expect(resolveDockPreviewTarget('src/a.ts', '')).toBeNull()
  })
})
