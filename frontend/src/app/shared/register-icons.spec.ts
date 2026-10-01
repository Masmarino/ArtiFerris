import { ApplicationInitStatus } from '@angular/core'
import { TestBed } from '@angular/core/testing'
import { IconRegistry } from '@masmarino/gabarit'
import { ARTIFERRIS_ICONS, provideArtiferrisIcons } from './register-icons'

describe('provideArtiferrisIcons', () => {
  it('registers only the static ARTIFERRIS_ICONS table, never a dynamically-constructed value', async () => {
    // Makes a future violation visible in review. The registry must only receive the static
    // ARTIFERRIS_ICONS table, never
    // an object rebuilt at runtime: reference equality proves it, where a spread or a map would
    // stay deep-equal.
    const registerAll = vi.fn()

    TestBed.configureTestingModule({
      providers: [provideArtiferrisIcons(), { provide: IconRegistry, useValue: { registerAll } }],
    })

    // Awaiting ApplicationInitStatus exercises the real initializer once, instead of calling the
    // callback by hand.
    await TestBed.inject(ApplicationInitStatus).donePromise

    expect(registerAll).toHaveBeenCalledTimes(1)
    expect(registerAll).toHaveBeenCalledWith(ARTIFERRIS_ICONS)
    expect(registerAll.mock.calls[0][0]).toBe(ARTIFERRIS_ICONS)
  })
})
