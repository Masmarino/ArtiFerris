import { TestBed } from '@angular/core/testing'
import { Router } from '@angular/router'
import { BLANK_ROUTES, setUpTestApp } from '../../testing/transloco-testing'
import { Security } from './security'

describe('Security', () => {
  beforeEach(async () => {
    await setUpTestApp({ imports: [Security], lang: 'en', routes: BLANK_ROUTES })
    await TestBed.inject(Router).navigateByUrl('/en/security')
  })

  async function render() {
    const fixture = TestBed.createComponent(Security)
    await fixture.whenStable()
    return fixture.nativeElement as HTMLElement
  }

  it('has one h1 and names the ten facts with the mechanism behind each', async () => {
    const root = await render()

    expect(root.querySelectorAll('h1')).toHaveLength(1)
    expect(root.querySelectorAll('.sec__row')).toHaveLength(10)
    expect(root.textContent).toContain('Argon2')
    expect(root.textContent).toContain('AES-256-GCM')
    expect(root.textContent).toContain('SECRETS_ENCRYPTION_KEY')
  })

  it('opens on the headers a real response carries, with what sets them', async () => {
    const root = await render()
    const output = root.querySelector('app-terminal pre')?.textContent ?? ''

    expect(output).toContain("content-security-policy: default-src 'self'")
    expect(output).toContain('x-frame-options: DENY')
    expect(output).toContain('x-content-type-options: nosniff')
    expect(output).toContain('referrer-policy: same-origin')
    // Some headers of a response through the proxy come from the proxy: the caption says which.
    expect(root.querySelector('app-terminal figcaption')?.textContent).toContain('reverse proxy')
  })

  it('points at the Security section of the README', async () => {
    const root = await render()

    expect(
      root.querySelector('a[href="https://github.com/Masmarino/ArtiFerris#sécurité"]'),
    ).not.toBeNull()
  })
})
