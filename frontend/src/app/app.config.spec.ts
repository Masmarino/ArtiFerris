import { TestBed } from '@angular/core/testing'
import { provideHttpClientTesting } from '@angular/common/http/testing'
import { TRANSLOCO_CONFIG } from '@jsverse/transloco'

vi.mock('@angular/core', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@angular/core')>()),
  isDevMode: () => false,
}))

describe('appConfig', () => {
  it('turns Transloco prodMode on when Angular is not in dev mode', async () => {
    const { appConfig } = await import('./app.config')
    TestBed.configureTestingModule({
      providers: [...appConfig.providers, provideHttpClientTesting()],
    })

    expect(TestBed.inject(TRANSLOCO_CONFIG).prodMode).toBe(true)
  })
})
