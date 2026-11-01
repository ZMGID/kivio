import { describe, expect, it } from 'vitest'
import { i18n } from '../../components/i18n'
import { formatScheduleTime, scheduleSummary } from './scheduleSummary'

describe('scheduleSummary', () => {
  it('renders daily times with leading zeroes and the selected language', () => {
    expect(scheduleSummary({ kind: 'daily', hour: 9, minute: 5 }, i18n.zh, 'zh'))
      .toBe(i18n.zh.chatSchedulesSummaryDaily.replace('{time}', '09:05'))
    expect(scheduleSummary({ kind: 'daily', hour: 9, minute: 5 }, i18n.en, 'en'))
      .toBe(i18n.en.chatSchedulesSummaryDaily.replace('{time}', '09:05'))
  })
  it('orders weekly days from Monday through Sunday without modifying the rule', () => {
    const weekdays = [0, 3, 1]
    expect(scheduleSummary({ kind: 'weekly', weekdays, hour: 8, minute: 30 }, i18n.zh, 'zh'))
      .toBe(i18n.zh.chatSchedulesSummaryWeekly
        .replace('{days}', [i18n.zh.chatSchedulesMonday, i18n.zh.chatSchedulesWednesday, i18n.zh.chatSchedulesSunday].join(i18n.zh.chatSchedulesWeekdaySeparator))
        .replace('{time}', '08:30'))
    expect(weekdays).toEqual([0, 3, 1])
  })
  it('renders all monthly days in order without changing the rule and preserves leap-day yearly schedules', () => {
    const days = [31, 1, 15]
    expect(scheduleSummary({ kind: 'monthly', days, hour: 7, minute: 5 }, i18n.zh, 'zh'))
      .toBe(i18n.zh.chatSchedulesSummaryMonthly
        .replace('{days}', ['1', '15', '31'].join(i18n.zh.chatSchedulesWeekdaySeparator))
        .replace('{time}', '07:05'))
    expect(days).toEqual([31, 1, 15])
    expect(scheduleSummary({ kind: 'yearly', month: 2, day: 29, hour: 9, minute: 0 }, i18n.zh, 'zh'))
      .toBe(i18n.zh.chatSchedulesSummaryYearly.replace('{month}', '2').replace('{day}', '29').replace('{time}', '09:00'))
    expect(scheduleSummary({ kind: 'yearly', month: 12, day: 31, hour: 23, minute: 59 }, i18n.en, 'en'))
      .toBe(i18n.en.chatSchedulesSummaryYearly.replace('{month}', '12').replace('{day}', '31').replace('{time}', '23:59'))
  })
  it('preserves interval and cron details instead of treating them as clock schedules', () => {
    expect(scheduleSummary({ kind: 'interval', minutes: 30 }, i18n.zh, 'zh'))
      .toBe(i18n.zh.chatSchedulesSummaryInterval.replace('{minutes}', '30'))
    expect(scheduleSummary({ kind: 'cron', expr: '0 9 * * 1-5' }, i18n.zh, 'zh'))
      .toBe(i18n.zh.chatSchedulesSummaryCron.replace('{expr}', '0 9 * * 1-5'))
  })
  it('interprets once timestamps as seconds in local time and distinguishes no next run', () => {
    const at = Math.floor(new Date(2026, 9, 2, 9, 0).getTime() / 1000)
    expect(scheduleSummary({ kind: 'once', at }, i18n.zh, 'zh'))
      .toBe(i18n.zh.chatSchedulesSummaryOnce.replace('{time}', '2026/10/02 09:00'))
    expect(formatScheduleTime(null, 'zh')).toBe('—')
  })
})
