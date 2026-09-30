import { t } from './i18n/translator'

// Plural rules differ by language (French treats 0 and 1 alike), and Gabarit's function labels
// cannot be overridden with an attribute.
export const formatResultsAnnouncement = (count: number): string =>
  t(count !== 1 ? 'format.results_other' : 'format.results_one', { count })

export const formatSuggestionsAnnouncement = (count: number): string =>
  count === 0
    ? t('format.suggestions_none')
    : t(count !== 1 ? 'format.suggestions_other' : 'format.suggestions_one', { count })

export const formatSelectedCount = (count: number): string =>
  t(count !== 1 ? 'format.selected_other' : 'format.selected_one', { count })

export const formatWeeklyDownloads = (count: number): string =>
  t(count >= 2 ? 'format.weeklyDownloads_other' : 'format.weeklyDownloads_one', {
    count: new Intl.NumberFormat(t('meta.locale')).format(count),
  })

const BYTE_UNIT_KEYS = ['b', 'kb', 'mb', 'gb', 'tb']

export function formatBytes(bytes: number): string {
  if (bytes <= 0) {
    return `0 ${t('format.bytes.b')}`
  }
  const exponent = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), BYTE_UNIT_KEYS.length - 1)
  const value = bytes / 1024 ** exponent
  return `${value.toFixed(exponent === 0 ? 0 : 1)} ${t(`format.bytes.${BYTE_UNIT_KEYS[exponent]}`)}`
}

const RELATIVE_UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ['year', 365 * 86400],
  ['month', 30 * 86400],
  ['day', 86400],
  ['hour', 3600],
  ['minute', 60],
]

export function formatRelativeDate(iso: string, now: Date = new Date()): string {
  const seconds = Math.min(0, Math.round((new Date(iso).getTime() - now.getTime()) / 1000))
  const formatter = new Intl.RelativeTimeFormat(t('meta.locale'), { numeric: 'auto' })
  for (const [unit, size] of RELATIVE_UNITS) {
    if (-seconds >= size) {
      return formatter.format(Math.trunc(seconds / size), unit)
    }
  }
  return t('format.justNow')
}
