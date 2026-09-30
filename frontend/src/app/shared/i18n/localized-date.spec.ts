import { setActiveLanguage } from './translator'
import { LocalizedDatePipe, formatLocalizedDate } from './localized-date'

describe('localized dates', () => {
  const iso = '2026-03-04T10:00:00Z'

  afterEach(() => setActiveLanguage('fr'))

  it.each([
    ['en', 'March'],
    ['fr', 'mars'],
    ['es', 'marzo'],
    ['it', 'marzo'],
    ['de', 'März'],
  ] as const)('writes the month in %s', (language, month) => {
    setActiveLanguage(language)

    expect(formatLocalizedDate(iso, 'longDate')).toContain(month)
  })

  it('leaves a missing date empty', () => {
    expect(formatLocalizedDate(null)).toBe('')
    expect(formatLocalizedDate(undefined)).toBe('')
    expect(formatLocalizedDate('')).toBe('')
  })

  it('backs the date pipe, with a null for a missing date', () => {
    setActiveLanguage('de')
    const pipe = new LocalizedDatePipe()

    expect(pipe.transform(iso, 'longDate')).toContain('März')
    expect(pipe.transform(null)).toBeNull()
  })
})
