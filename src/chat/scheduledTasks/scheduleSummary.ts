import type { ScheduleRule } from '../../api/scheduledTaskContracts'
import type { I18n, Lang } from '../../components/i18n'

export function weekdayLabels(t: I18n): string[] {
  return [t.chatSchedulesSunday, t.chatSchedulesMonday, t.chatSchedulesTuesday, t.chatSchedulesWednesday,
    t.chatSchedulesThursday, t.chatSchedulesFriday, t.chatSchedulesSaturday]
}

export function formatScheduleTime(at: number | null, lang: Lang): string {
  if (at === null) return '—'
  return new Date(at * 1000).toLocaleString(lang === 'zh' ? 'zh-CN' : 'en-US', {
    year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false,
  })
}

export function scheduleSummary(schedule: ScheduleRule, t: I18n, lang: Lang): string {
  switch (schedule.kind) {
    case 'once': return t.chatSchedulesSummaryOnce.replace('{time}', formatScheduleTime(schedule.at, lang))
    case 'interval': return t.chatSchedulesSummaryInterval.replace('{minutes}', String(schedule.minutes))
    case 'cron': return t.chatSchedulesSummaryCron.replace('{expr}', schedule.expr)
    case 'daily': return t.chatSchedulesSummaryDaily.replace('{time}', clockTime(schedule.hour, schedule.minute))
    case 'monthly': return t.chatSchedulesSummaryMonthly
      .replace('{days}', [...schedule.days].sort((a, b) => a - b).join(t.chatSchedulesWeekdaySeparator))
      .replace('{time}', clockTime(schedule.hour, schedule.minute))
    case 'yearly': return t.chatSchedulesSummaryYearly
      .replace('{month}', String(schedule.month)).replace('{day}', String(schedule.day))
      .replace('{time}', clockTime(schedule.hour, schedule.minute))
    case 'weekly': return t.chatSchedulesSummaryWeekly
      .replace('{days}', [...schedule.weekdays].sort((a, b) => ((a + 6) % 7) - ((b + 6) % 7))
        .map(day => weekdayLabels(t)[day]).join(t.chatSchedulesWeekdaySeparator))
      .replace('{time}', clockTime(schedule.hour, schedule.minute))
  }
}

export function clockTime(hour: number, minute: number): string {
  return `${String(hour).padStart(2, '0')}:${String(minute).padStart(2, '0')}`
}

export function friendlyScheduleTime(at: number | null, t: I18n, lang: Lang, now = new Date()): string {
  if (at === null) return '—'
  const date = new Date(at * 1000)
  const time = clockTime(date.getHours(), date.getMinutes())
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate())
  const day = new Date(date.getFullYear(), date.getMonth(), date.getDate())
  const tomorrow = new Date(start)
  tomorrow.setDate(tomorrow.getDate() + 1)
  const nextWeek = new Date(start)
  nextWeek.setDate(nextWeek.getDate() + 7)
  if (+day === +start) return `${t.chatSchedulesToday} ${time}`
  if (+day === +tomorrow) return `${t.chatSchedulesTomorrow} ${time}`
  if (day > start && day < nextWeek) return `${weekdayLabels(t)[date.getDay()]} ${time}`
  const dateLabel = lang === 'zh'
    ? `${date.getFullYear() !== now.getFullYear() ? `${date.getFullYear()}年` : ''}${date.getMonth() + 1}月${date.getDate()}日`
    : date.toLocaleDateString('en-US', {
      ...(date.getFullYear() !== now.getFullYear() ? { year: 'numeric' as const } : {}),
      month: 'short', day: 'numeric',
    })
  return `${dateLabel} ${time}`
}
