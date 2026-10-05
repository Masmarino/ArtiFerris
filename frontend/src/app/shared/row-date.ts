import { formatDateTime, formatRelativeTime } from '@masmarino/gabarit/format'
import { activeLocale, t } from './i18n/translator'

/** A date in a list row: relative up to a month, then the day; the exact time goes in the tooltip. */
export interface RowDate {
  iso: string
  label: string
  title: string
}

const RELATIVE_OPTIONS = { style: 'short', maxUnit: 'day', absoluteAfterDays: 30 } as const
const ABSOLUTE_OPTIONS = {
  day: '2-digit',
  month: '2-digit',
  year: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
} as const
const DAY_OPTIONS = { day: '2-digit', month: '2-digit', year: 'numeric' } as const

/**
 * `agoKey` and `onKey` name the sentence for each form, so each language orders it its own way:
 * "créé il y a 3 j" (`{{ when }}`) but "créé le 12/08/2026" (`{{ date }}`).
 */
export function rowDate(iso: string, now: Date, agoKey: string, onKey: string): RowDate {
  const locale = activeLocale()
  const relative = formatRelativeTime(iso, locale, now, RELATIVE_OPTIONS)
  const title = formatDateTime(iso, locale, ABSOLUTE_OPTIONS)
  const label = /^\d/.test(relative)
    ? t(onKey, { date: formatDateTime(iso, locale, DAY_OPTIONS) })
    : t(agoKey, { when: relative })
  return { iso, label, title }
}

/** The exact time, for a sentence that gives it in full ("expire le 07/10/2026 05:50"). */
export const exactTime = (iso: string): string =>
  formatDateTime(iso, activeLocale(), ABSOLUTE_OPTIONS)
