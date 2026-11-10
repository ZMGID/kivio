import { createFileTreeIconResolver } from '@pierre/trees'

type FileKindVisual = {
  label: string
  iconClass: string
  wellClass: string
}

export const fileIconResolver = createFileTreeIconResolver('complete')
export const fileIconColors = new Map(Object.entries({
  'astro vite': 'text-purple-600 dark:text-purple-400',
  'babel browserslist javascript': 'text-yellow-600 dark:text-yellow-300',
  'bash markdown svgo vue': 'text-green-600 dark:text-green-400',
  'biome c cpp css docker eslint python typescript vscode webpack': 'text-blue-600 dark:text-blue-400',
  'bootstrap sass stylelint terraform wasm': 'text-violet-600 dark:text-violet-400',
  'bun': 'text-stone-600 dark:text-stone-400',
  'claude html json rust svg swift zig zip': 'text-orange-600 dark:text-orange-400',
  'database': 'text-purple-600 dark:text-purple-400',
  'git graphql image npm postcss ruby svelte yml': 'text-rose-600 dark:text-rose-400',
  'go mcp oxc prettier react table tailwind': 'text-cyan-600 dark:text-cyan-400',
}).flatMap(([tokens, color]) => tokens.split(' ').map<[string, string]>(token => [token, color])))

const CODE_EXTS = new Set([
  'ts', 'tsx', 'mts', 'cts', 'js', 'jsx', 'mjs', 'cjs',
  'py', 'rs', 'go', 'java', 'kt', 'kts', 'c', 'h', 'cc', 'cpp', 'cxx', 'hpp',
  'rb', 'php', 'swift', 'html', 'htm', 'css', 'scss', 'less', 'vue', 'svelte',
  'sh', 'bash', 'zsh', 'ps1', 'bat', 'sql',
])

function extensionOf(name: string): string {
  const slash = Math.max(name.lastIndexOf('/'), name.lastIndexOf('\\'))
  const base = slash >= 0 ? name.slice(slash + 1) : name
  const dot = base.lastIndexOf('.')
  if (dot <= 0 || dot === base.length - 1) return ''
  return base.slice(dot + 1).toLowerCase()
}

export function folderKindVisual(): FileKindVisual {
  return {
    label: 'FOLDER',
    iconClass: 'text-[var(--text-muted)]',
    wellClass: 'bg-neutral-500/10',
  }
}

export function fileKindVisual(name: string): FileKindVisual {
  const ext = extensionOf(name)
  const label = ext ? ext.toUpperCase() : 'FILE'
  if (ext === 'pdf') {
    return { label, iconClass: 'text-red-500 dark:text-red-400', wellClass: 'bg-red-500/10 dark:bg-red-400/15' }
  }
  if (ext === 'doc' || ext === 'docx') {
    return { label, iconClass: 'text-blue-600 dark:text-blue-400', wellClass: 'bg-blue-500/10 dark:bg-blue-400/15' }
  }
  if (ext === 'xls' || ext === 'xlsx' || ext === 'xlsm' || ext === 'csv' || ext === 'tsv') {
    return { label, iconClass: 'text-emerald-600 dark:text-emerald-400', wellClass: 'bg-emerald-500/10 dark:bg-emerald-400/15' }
  }
  if (ext === 'ppt' || ext === 'pptx') {
    return { label, iconClass: 'text-orange-500 dark:text-orange-400', wellClass: 'bg-orange-500/10 dark:bg-orange-400/15' }
  }
  if (ext === 'zip' || ext === 'tar' || ext === 'gz' || ext === '7z' || ext === 'rar') {
    return { label, iconClass: 'text-amber-600 dark:text-amber-400', wellClass: 'bg-amber-500/10 dark:bg-amber-400/15' }
  }
  if (ext === 'json' || ext === 'jsonc' || ext === 'yaml' || ext === 'yml' || ext === 'toml') {
    return { label, iconClass: 'text-amber-500 dark:text-amber-400', wellClass: 'bg-amber-500/10 dark:bg-amber-400/15' }
  }
  if (ext === 'mp3' || ext === 'wav' || ext === 'm4a' || ext === 'flac' || ext === 'ogg' || ext === 'aac') {
    return { label, iconClass: 'text-violet-500 dark:text-violet-400', wellClass: 'bg-violet-500/10 dark:bg-violet-400/15' }
  }
  if (ext === 'mp4' || ext === 'mov' || ext === 'webm' || ext === 'mkv' || ext === 'avi') {
    return { label, iconClass: 'text-fuchsia-500 dark:text-fuchsia-400', wellClass: 'bg-fuchsia-500/10 dark:bg-fuchsia-400/15' }
  }
  if (CODE_EXTS.has(ext)) {
    return { label, iconClass: 'text-sky-500 dark:text-sky-400', wellClass: 'bg-sky-500/10 dark:bg-sky-400/15' }
  }
  if (ext === 'md' || ext === 'markdown' || ext === 'txt' || ext === 'log') {
    return { label, iconClass: 'text-neutral-500', wellClass: 'bg-neutral-500/10' }
  }
  return { label, iconClass: 'text-neutral-500', wellClass: 'bg-neutral-500/10' }
}
