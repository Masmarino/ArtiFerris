import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import {
  FALLBACK_LANGUAGE,
  LANGUAGE_LOCALES,
  SUPPORTED_LANGUAGES,
  detectBrowserLanguage,
  pickLanguage,
} from './languages'

describe('pickLanguage', () => {
  it.each([
    [['fr'], 'fr'],
    [['fr-FR'], 'fr'],
    [['fr-CA'], 'fr'],
    [['en-GB'], 'en'],
    [['es-MX'], 'es'],
    [['it'], 'it'],
    [['de-AT'], 'de'],
    [['DE_de'], 'de'],
  ])('maps %j to %s', (preferred, expected) => {
    expect(pickLanguage(preferred)).toBe(expected)
  })

  it('takes the first translated language, in order of preference', () => {
    expect(pickLanguage(['ja', 'pt-BR', 'de', 'fr'])).toBe('de')
  })

  it('falls back to English when no language is translated', () => {
    expect(pickLanguage(['ja', 'zh-CN'])).toBe(FALLBACK_LANGUAGE)
    expect(FALLBACK_LANGUAGE).toBe('en')
  })

  it('falls back to English without any preference', () => {
    expect(pickLanguage([])).toBe('en')
    expect(pickLanguage([''])).toBe('en')
  })

  it('does not mistake an inherited property for a language', () => {
    expect(pickLanguage(['constructor', 'toString'])).toBe('en')
  })
})

describe('detectBrowserLanguage', () => {
  afterEach(() => vi.restoreAllMocks())

  it('reads navigator.languages', () => {
    vi.spyOn(navigator, 'languages', 'get').mockReturnValue(['it-IT', 'en'])

    expect(detectBrowserLanguage()).toBe('it')
  })
})

describe('language locales', () => {
  it.each(SUPPORTED_LANGUAGES)('matches meta.locale of %s.json', (language) => {
    const file = join(process.cwd(), 'public', 'i18n', `${language}.json`)
    const dictionary = JSON.parse(readFileSync(file, 'utf8')) as { meta: { locale: string } }

    expect(dictionary.meta.locale).toBe(LANGUAGE_LOCALES[language])
  })
})
