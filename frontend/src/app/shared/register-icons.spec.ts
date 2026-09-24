import { ApplicationInitStatus } from '@angular/core'
import { TestBed } from '@angular/core/testing'
import { IconRegistry } from '@masmarino/gabarit'
import { ARTIFERRIS_ICONS, provideArtiferrisIcons } from './register-icons'

describe('provideArtiferrisIcons', () => {
  it('registers only the static ARTIFERRIS_ICONS table, never a dynamically-constructed value', async () => {
    // This test exists to make a future violation visible at review time, not to catch a bug that
    // exists today. `Icon` in @masmarino/gabarit renders whatever markup IconRegistry hands it via
    // bypassSecurityTrustHtml, so the registry must only ever receive the compile-time-static
    // ARTIFERRIS_ICONS table (B-32) — never an object rebuilt from a variable, a template
    // interpolation, or runtime/API data. Reference equality is what actually proves that: a
    // spread, map, or merge would produce a deep-equal-but-different object and still pass a
    // looser check.
    const registerAll = vi.fn()

    TestBed.configureTestingModule({
      providers: [provideArtiferrisIcons(), { provide: IconRegistry, useValue: { registerAll } }],
    })

    // provideAppInitializer's callback runs as part of Angular's normal app-initializer sequence;
    // TestBed runs that sequence itself once the environment injector is touched, so awaiting
    // ApplicationInitStatus's donePromise (rather than reaching in and invoking the callback a
    // second time by hand) is what exercises the real registration path exactly once.
    await TestBed.inject(ApplicationInitStatus).donePromise

    expect(registerAll).toHaveBeenCalledTimes(1)
    expect(registerAll).toHaveBeenCalledWith(ARTIFERRIS_ICONS)
    expect(registerAll.mock.calls[0][0]).toBe(ARTIFERRIS_ICONS)
  })
})
