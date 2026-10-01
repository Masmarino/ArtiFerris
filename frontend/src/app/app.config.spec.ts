import { TestBed } from '@angular/core/testing'
import { provideHttpClientTesting } from '@angular/common/http/testing'
import { TRANSLOCO_CONFIG } from '@jsverse/transloco'

vi.mock('@angular/core', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@angular/core')>()),
  isDevMode: () => false,
}))

describe('appConfig', () => {
  it('turns Transloco prodMode on when Angular is not in dev mode', async () => {
    // A fresh module graph for the dynamic import: a shared worker that already loaded the real
    // '@angular/core'
    // would ignore this file's vi.mock (passes locally, flaky in CI).
    vi.resetModules()
    const { appConfig } = await import('./app.config')
    TestBed.configureTestingModule({
      providers: [...appConfig.providers, provideHttpClientTesting()],
    })

    expect(TestBed.inject(TRANSLOCO_CONFIG).prodMode).toBe(true)
  })
})
