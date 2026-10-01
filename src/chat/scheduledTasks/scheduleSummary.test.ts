import { describe, expect, it } from 'vitest'
import { i18n } from '../../components/i18n'
import { formatScheduleTime, scheduleSummary } from './scheduleSummary'

describe('scheduleSummary', () => {
  it('renders daily times with leading zeroes and the selected language', () => {
    expect(scheduleSummary({ kind: 'daily', hour: 9, minute: 5 }, i18n.zh, 'zh')).toBe('每天 09:05')
    expect(scheduleSummary({ kind: 'daily', hour: 9, minute: 5 }, i18n.en, 'en')).toBe('Daily at 09:05')
  })
  it('orders weekly days from Monday through Sunday without modifying the rule', () => {
    const weekdays = [0, 3, 1]
    expect(scheduleSummary({ kind: 'weekly', weekdays, hour: 8, minute: 30 }, i18n.zh, 'zh'))
      .toBe('每周一、周三、周日 08:30')
    expect(weekdays).toEqual([0, 3, 1])
  })
  it('renders all monthly days in order without changing the rule and preserves leap-day yearly schedules', () => {
    const days = [31, 1, 15]
    expect(scheduleSummary({ kind: 'monthly', days, hour: 7, minute: 5 }, i18n.zh, 'zh')).toBe('每月 1、15、31日 07:05')
    expect(days).toEqual([31, 1, 15])
    expect(scheduleSummary({ kind: 'yearly', month: 2, day: 29, hour: 9, minute: 0 }, i18n.zh, 'zh')).toBe('每年 2月29日 09:00')
    expect(scheduleSummary({ kind: 'yearly', month: 12, day: 31, hour: 23, minute: 59 }, i18n.en, 'en')).toBe('Yearly on 12/31 at 23:59')
  })
  it('preserves interval and cron details instead of treating them as clock schedules', () => {
    expect(scheduleSummary({ kind: 'interval', minutes: 30 }, i18n.zh, 'zh')).toBe('每 30 分钟')
    expect(scheduleSummary({ kind: 'cron', expr: '0 9 * * 1-5' }, i18n.zh, 'zh')).toBe('Cron 0 9 * * 1-5')
  })
  it('interprets once timestamps as seconds in local time and distinguishes no next run', () => {
    const at = Math.floor(new Date(2026, 9, 2, 9, 0).getTime() / 1000)
    expect(scheduleSummary({ kind: 'once', at }, i18n.zh, 'zh')).toBe('2026/10/02 09:00 一次')
    expect(formatScheduleTime(null, 'zh')).toBe('—')
  })
})
