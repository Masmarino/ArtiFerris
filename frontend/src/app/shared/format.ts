// French pluralization for Gabarit's function-typed label inputs — their English defaults can't be overridden with a plain attribute.
export const formatResultsAnnouncement = (count: number): string =>
  `${count} résultat${count !== 1 ? 's' : ''}`

export const formatSuggestionsAnnouncement = (count: number): string =>
  count === 0
    ? 'Aucune suggestion'
    : `${count} suggestion${count !== 1 ? 's' : ''} disponible${count !== 1 ? 's' : ''}`

export const formatSelectedCount = (count: number): string =>
  `${count} sélectionné${count !== 1 ? 's' : ''}`

const COUNT_FORMAT = new Intl.NumberFormat('fr-FR')

/** "1 234 téléchargements cette semaine". French keeps 0 and 1 singular. */
export const formatWeeklyDownloads = (count: number): string =>
  `${COUNT_FORMAT.format(count)} téléchargement${count >= 2 ? 's' : ''} cette semaine`

const BYTE_UNITS = ['o', 'Ko', 'Mo', 'Go', 'To']

export function formatBytes(bytes: number): string {
  if (bytes <= 0) {
    return '0 o'
  }
  const exponent = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), BYTE_UNITS.length - 1)
  const value = bytes / 1024 ** exponent
  return `${value.toFixed(exponent === 0 ? 0 : 1)} ${BYTE_UNITS[exponent]}`
}

const RELATIVE_UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ['year', 365 * 86400],
  ['month', 30 * 86400],
  ['day', 86400],
  ['hour', 3600],
  ['minute', 60],
]

/** "il y a 3 jours", "hier", "à l'instant". A date slightly in the future (clock skew) counts as now. */
export function formatRelativeDate(iso: string, now: Date = new Date()): string {
  const seconds = Math.min(0, Math.round((new Date(iso).getTime() - now.getTime()) / 1000))
  const formatter = new Intl.RelativeTimeFormat('fr', { numeric: 'auto' })
  for (const [unit, size] of RELATIVE_UNITS) {
    if (-seconds >= size) {
      return formatter.format(Math.trunc(seconds / size), unit)
    }
  }
  return "à l'instant"
}
