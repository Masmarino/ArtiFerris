import { LOCALE_ID, ApplicationInitStatus } from '@angular/core'
import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { TranslocoService } from '@jsverse/transloco'
import { appConfig } from './app.config'

describe('appConfig language', () => {
  afterEach(() => {
    vi.restoreAllMocks()
    document.documentElement.lang = ''
  })

  async function start(browserLanguages: string[]) {
    vi.spyOn(navigator, 'languages', 'get').mockReturnValue(browserLanguages)
    TestBed.configureTestingModule({
      providers: [...appConfig.providers, provideHttpClientTesting()],
    })
    const http = TestBed.inject(HttpTestingController)
    const initialized = TestBed.inject(ApplicationInitStatus).donePromise
    return { http, initialized }
  }

  it.each([
    [['fr-CA', 'en'], 'fr', 'fr-FR'],
    [['de-AT'], 'de', 'de-DE'],
    [['es-MX'], 'es', 'es-ES'],
    [['it'], 'it', 'it-IT'],
    [['en-GB'], 'en', 'en-US'],
    [['ja', 'zh-CN'], 'en', 'en-US'],
  ])('starts in the language of a browser set to %j: %s', async (languages, language, locale) => {
    const { http, initialized } = await start(languages)

    http.expectOne(`/i18n/${language}.json`).flush({ meta: { locale } })
    await initialized

    expect(TestBed.inject(TranslocoService).getActiveLang()).toBe(language)
    expect(TestBed.inject(LOCALE_ID)).toBe(locale)
    expect(document.documentElement.lang).toBe(language)
  })
})
