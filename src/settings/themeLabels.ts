import type { Lang } from '../components/i18n'
import type { ThemeDefinition } from '../theme/types'

const BUILTIN_ZH_LABELS: Record<string, string> = {
  neutral: '中性', warm: '暖白', cool: '冷白',
  graphite: '石墨', blossom: '樱粉', grove: '森林',
  ocean: '海湾', ember: '余烬', iris: '鸢尾',
  white: '纯白', nord: '北欧 Nord', solarized: '日晒 Solarized',
}

/** 主题显示名：内置主题在中文界面使用中文名，自定义主题始终使用用户起的名字。 */
export function themeDisplayName(theme: ThemeDefinition, lang: Lang, builtin: boolean): string {
  return lang === 'zh' && builtin ? BUILTIN_ZH_LABELS[theme.id] ?? theme.name : theme.name
}
