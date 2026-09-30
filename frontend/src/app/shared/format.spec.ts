import {
  formatBytes,
  formatRelativeDate,
  formatResultsAnnouncement,
  formatSelectedCount,
  formatSuggestionsAnnouncement,
  formatWeeklyDownloads,
} from './format'

describe('formatBytes', () => {
  it('renders zero and negative values as 0 o', () => {
    expect(formatBytes(0)).toBe('0 o')
    expect(formatBytes(-5)).toBe('0 o')
  })

  it('renders bytes below 1024 without a decimal', () => {
    expect(formatBytes(512)).toBe('512 o')
  })

  it('renders larger values with the closest unit and one decimal', () => {
    expect(formatBytes(1024)).toBe('1.0 Ko')
    expect(formatBytes(1536)).toBe('1.5 Ko')
    expect(formatBytes(1024 * 1024 * 3)).toBe('3.0 Mo')
  })
})

// A live-region announcement (RGAA 7.5): a plural mistake is only heard by screen readers, so 1
// gets its own case.
describe('formatResultsAnnouncement', () => {
  it('pluralizes zero results', () => {
    expect(formatResultsAnnouncement(0)).toBe('0 résultats')
  })

  it('keeps one result singular', () => {
    expect(formatResultsAnnouncement(1)).toBe('1 résultat')
  })

  it('pluralizes several results', () => {
    expect(formatResultsAnnouncement(2)).toBe('2 résultats')
    expect(formatResultsAnnouncement(17)).toBe('17 résultats')
  })
})

describe('formatSuggestionsAnnouncement', () => {
  it('says there is no suggestion', () => {
    expect(formatSuggestionsAnnouncement(0)).toBe('Aucune suggestion')
  })

  it('keeps one suggestion singular', () => {
    expect(formatSuggestionsAnnouncement(1)).toBe('1 suggestion disponible')
  })

  it('pluralizes several suggestions', () => {
    expect(formatSuggestionsAnnouncement(8)).toBe('8 suggestions disponibles')
  })
})

// Drives the selectedCountLabel of gbt-select: same singular/plural boundary.
describe('formatSelectedCount', () => {
  it('pluralizes zero selected', () => {
    expect(formatSelectedCount(0)).toBe('0 sélectionnés')
  })

  it('keeps one selected singular', () => {
    expect(formatSelectedCount(1)).toBe('1 sélectionné')
  })

  it('pluralizes several selected', () => {
    expect(formatSelectedCount(2)).toBe('2 sélectionnés')
    expect(formatSelectedCount(9)).toBe('9 sélectionnés')
  })
})

describe('formatWeeklyDownloads', () => {
  it('keeps zero and one singular, as French does', () => {
    expect(formatWeeklyDownloads(0)).toBe('0 téléchargement cette semaine')
    expect(formatWeeklyDownloads(1)).toBe('1 téléchargement cette semaine')
  })

  it('pluralizes from two', () => {
    expect(formatWeeklyDownloads(2)).toBe('2 téléchargements cette semaine')
  })

  it('separates thousands with a narrow no-break space', () => {
    expect(formatWeeklyDownloads(999)).toBe('999 téléchargements cette semaine')
    expect(formatWeeklyDownloads(1234)).toBe('1\u202f234 téléchargements cette semaine')
    expect(formatWeeklyDownloads(1_250_000)).toBe(
      '1\u202f250\u202f000 téléchargements cette semaine',
    )
  })
})

describe('formatRelativeDate', () => {
  const now = new Date('2026-09-24T12:00:00Z')

  it('says "à l\'instant" under a minute, and for a date slightly in the future', () => {
    expect(formatRelativeDate('2026-09-24T11:59:30Z', now)).toBe("à l'instant")
    expect(formatRelativeDate('2026-09-24T12:00:20Z', now)).toBe("à l'instant")
  })

  it('picks the largest fitting unit', () => {
    expect(formatRelativeDate('2026-09-24T11:55:00Z', now)).toBe('il y a 5 minutes')
    expect(formatRelativeDate('2026-09-24T09:00:00Z', now)).toBe('il y a 3 heures')
    expect(formatRelativeDate('2026-09-21T12:00:00Z', now)).toBe('il y a 3 jours')
    expect(formatRelativeDate('2026-06-24T12:00:00Z', now)).toBe('il y a 3 mois')
    expect(formatRelativeDate('2024-09-24T12:00:00Z', now)).toBe('il y a 2 ans')
  })

  it('uses the natural wording for yesterday', () => {
    expect(formatRelativeDate('2026-09-23T12:00:00Z', now)).toBe('hier')
  })
})
