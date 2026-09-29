import { TestBed } from '@angular/core/testing'
import { provideHttpClientTesting } from '@angular/common/http/testing'
import { TRANSLOCO_CONFIG } from '@jsverse/transloco'

vi.mock('@angular/core', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@angular/core')>()),
  isDevMode: () => false,
}))

describe('appConfig', () => {
  it('turns Transloco prodMode on when Angular is not in dev mode', async () => {
    // Forces a fresh module graph for this dynamic import: without it, a shared worker that
    // already loaded the real (unmocked) '@angular/core' for an earlier test file can hand
    // app.config the real isDevMode instead of this file's hoisted vi.mock override — passes
    // locally, flakes in CI where file-to-worker scheduling differs.
    vi.resetModules()
    const { appConfig } = await import('./app.config')
    TestBed.configureTestingModule({
      providers: [...appConfig.providers, provideHttpClientTesting()],
    })

    expect(TestBed.inject(TRANSLOCO_CONFIG).prodMode).toBe(true)
  })
})
