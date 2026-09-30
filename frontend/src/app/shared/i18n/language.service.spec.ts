import { computed } from '@angular/core'
import { TestBed } from '@angular/core/testing'
import { Injectable } from '@angular/core'
import {
  Translation,
  TranslocoLoader,
  TranslocoService,
  provideTransloco,
} from '@jsverse/transloco'
import { Observable, of } from 'rxjs'
import { LanguageService } from './language.service'
import { activeLocale, t } from './translator'
import { formatLocalizedDate } from './localized-date'

const DICTIONARIES: Record<string, Translation> = {
  fr: { common: { close: 'Fermer' } },
  en: { common: { close: 'Close' } },
  de: { common: { close: 'Schließen' } },
}

@Injectable()
class MapLoader implements TranslocoLoader {
  getTranslation(lang: string): Observable<Translation> {
    return of(DICTIONARIES[lang])
  }
}

describe('LanguageService', () => {
  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [
        provideTransloco({
          config: { availableLangs: ['fr', 'en', 'de'], defaultLang: 'fr', prodMode: false },
          loader: MapLoader,
        }),
      ],
    })
  })

  afterEach(() => {
    document.documentElement.lang = ''
  })

  it('switches the language the translator and the transloco service use', async () => {
    const service = TestBed.inject(LanguageService)

    await service.use('en')

    expect(service.language()).toBe('en')
    expect(TestBed.inject(TranslocoService).getActiveLang()).toBe('en')
    expect(t('common.close')).toBe('Close')
  })

  it('updates <html lang>', async () => {
    await TestBed.inject(LanguageService).use('de')

    expect(document.documentElement.lang).toBe('de')
  })

  it('re-evaluates what was computed from t() when the language changes', async () => {
    const service = TestBed.inject(LanguageService)
    await service.use('fr')
    const label = TestBed.runInInjectionContext(() => computed(() => t('common.close')))
    expect(label()).toBe('Fermer')

    await service.use('de')

    expect(label()).toBe('Schließen')
  })

  it('formats dates and numbers with the locale of the new language', async () => {
    const service = TestBed.inject(LanguageService)
    await service.use('en')
    expect(activeLocale()).toBe('en-US')
    expect(formatLocalizedDate('2026-03-04T10:00:00Z', 'fullDate')).toContain('Wednesday')

    await service.use('fr')

    expect(activeLocale()).toBe('fr-FR')
    expect(formatLocalizedDate('2026-03-04T10:00:00Z', 'fullDate')).toContain('mercredi')
  })
})
