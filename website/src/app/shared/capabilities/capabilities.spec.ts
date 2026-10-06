import { TestBed } from '@angular/core/testing'
import { setUpTestApp } from '../../testing/transloco-testing'
import { Capabilities } from './capabilities'

describe('Capabilities', () => {
  async function render(lang: 'en' | 'fr' = 'en') {
    await setUpTestApp({ imports: [Capabilities], lang })
    const fixture = TestBed.createComponent(Capabilities)
    await fixture.whenStable()
    return fixture
  }

  it('shows eight tiles, each with a heading and a fragment of interface', async () => {
    const root: HTMLElement = (await render()).nativeElement
    const tiles = root.querySelectorAll('.tile')
    expect(tiles).toHaveLength(8)
    for (const tile of Array.from(tiles)) {
      expect(tile.querySelector('h3')?.textContent?.trim().length).toBeGreaterThan(0)
      expect(tile.querySelector('.frag')).not.toBeNull()
    }
  })

  it('copies the install command of the package tile and lets it be undone', async () => {
    const fixture = await render('fr')
    const root: HTMLElement = fixture.nativeElement

    expect(root.querySelector('.tile--package .diff')?.textContent).toContain(
      'npm install left-pad',
    )
    const copy = root.querySelector<HTMLButtonElement>('.frag__button')!
    expect(copy.textContent?.trim()).toBe('Copier')
    copy.click()
    await fixture.whenStable()
    expect(root.querySelector('.applied')?.textContent).toContain('Copiée')
    expect(root.querySelector('.frag--review')?.classList.contains('is-applied')).toBe(true)

    root.querySelector<HTMLButtonElement>('.frag__reset')!.click()
    await fixture.whenStable()
    expect(root.querySelector('.frag__button')).not.toBeNull()
  })

  it('states the facts of the documentation: the three rights, the three types, the search groups', async () => {
    const root: HTMLElement = (await render()).nativeElement

    expect(
      Array.from(root.querySelectorAll('.roles thead th.mono'), (th) => th.textContent),
    ).toEqual(['read', 'write', 'admin'])
    expect(root.querySelector('.tile--types')?.textContent).toContain('registry.npmjs.org')
    expect(root.querySelector('.tile--tenants')?.textContent).toContain('acme.artiferris.pro')
    const marks = Array.from(root.querySelectorAll('.roles tbody tr')).map((row) =>
      Array.from(row.querySelectorAll('.roles__mark')).map((m) => m.classList.contains('is-on')),
    )
    expect(marks).toEqual([
      [true, true, true],
      [false, true, true],
      [false, false, true],
    ])
    expect(root.querySelectorAll('.search__results li')).toHaveLength(3)
    // Package figures, scan severities, quota and retention, factors of an account.
    expect(root.querySelectorAll('.factor')).toHaveLength(11)
  })

  describe('as a carousel', () => {
    it('follows the numbers of the architecture plan, the tile with no number last', async () => {
      const root: HTMLElement = (await render('fr')).nativeElement
      const numbers = Array.from(root.querySelectorAll('.carousel__track > .tile')).map(
        (tile) => tile.querySelector('.tile__no')?.textContent?.trim() ?? null,
      )

      expect(numbers).toEqual(['1', '2', '3', '4', '5', '6', '7', null])
    })

    const buttons = (root: HTMLElement, selector: string) =>
      Array.from(root.querySelectorAll<HTMLButtonElement>(selector))

    it('is a labelled carousel region whose track can be focused and whose tiles are numbered slides', async () => {
      const root: HTMLElement = (await render('fr')).nativeElement
      const region = root.querySelector('.carousel')!

      expect(region.getAttribute('role')).toBe('region')
      expect(region.getAttribute('aria-roledescription')).toBe('carousel')
      expect(region.getAttribute('aria-label')).toBe("Extraits de l'interface")
      expect(root.querySelector('.carousel__track')?.getAttribute('tabindex')).toBe('0')
      const slides = Array.from(root.querySelectorAll('.carousel__track > .tile'))
      expect(slides.map((slide) => slide.getAttribute('aria-label'))).toEqual(
        [1, 2, 3, 4, 5, 6, 7, 8].map((n) => `${n} sur 8`),
      )
      expect(
        slides.every((slide) => slide.getAttribute('aria-roledescription') === 'diapositive'),
      ).toBe(true)
    })

    it('has one dot per tile, named after the tile, the first one current', async () => {
      const root: HTMLElement = (await render('fr')).nativeElement
      const dots = buttons(root, '.carousel__dot')

      expect(dots).toHaveLength(8)
      expect(dots[0].getAttribute('aria-label')).toBe('Trois droits par dépôt')
      expect(dots.map((dot) => dot.getAttribute('aria-current'))).toEqual([
        'true',
        ...Array(7).fill(null),
      ])
    })

    it('starts at the first tile, so there is nothing before it', async () => {
      const root: HTMLElement = (await render('fr')).nativeElement

      expect(buttons(root, '.carousel__nav')[0].disabled).toBe(true)
      expect(buttons(root, '.carousel__nav')[1].disabled).toBe(false)
    })

    it('moves to the next and previous tile, and says where it is', async () => {
      const fixture = await render('fr')
      const root: HTMLElement = fixture.nativeElement
      const [previous, next] = buttons(root, '.carousel__nav')

      next.click()
      await fixture.whenStable()
      expect(root.querySelector('.carousel__count')?.textContent?.trim()).toBe(
        'Fonctionnalité 2 sur 8',
      )
      expect(buttons(root, '.carousel__dot')[1].getAttribute('aria-current')).toBe('true')
      expect(previous.disabled).toBe(false)

      previous.click()
      await fixture.whenStable()
      expect(root.querySelector('.carousel__count')?.textContent?.trim()).toBe(
        'Fonctionnalité 1 sur 8',
      )
    })

    it('goes straight to the tile of a dot, and stops at the last one', async () => {
      const fixture = await render('fr')
      const root: HTMLElement = fixture.nativeElement

      buttons(root, '.carousel__dot')[7].click()
      await fixture.whenStable()

      expect(root.querySelector('.carousel__count')?.textContent?.trim()).toBe(
        'Fonctionnalité 8 sur 8',
      )
      expect(buttons(root, '.carousel__nav')[1].disabled).toBe(true)
    })

    it('takes the arrow keys only when the track itself has the focus', async () => {
      const fixture = await render('fr')
      const root: HTMLElement = fixture.nativeElement
      const track = root.querySelector<HTMLElement>('.carousel__track')!

      root
        .querySelector('.frag__button')!
        .dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }))
      await fixture.whenStable()
      expect(root.querySelector('.carousel__count')?.textContent?.trim()).toBe(
        'Fonctionnalité 1 sur 8',
      )

      track.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }),
      )
      await fixture.whenStable()
      expect(root.querySelector('.carousel__count')?.textContent?.trim()).toBe(
        'Fonctionnalité 2 sur 8',
      )
    })
  })
})
