import { TestBed } from '@angular/core/testing'
import { Router } from '@angular/router'
import { BLANK_ROUTES, setUpTestApp } from '../../testing/transloco-testing'
import { InstallPage } from './install'

describe('InstallPage', () => {
  beforeEach(async () => {
    await setUpTestApp({ imports: [InstallPage], lang: 'en', routes: BLANK_ROUTES })
    await TestBed.inject(Router).navigateByUrl('/en/install')
  })

  async function render() {
    const fixture = TestBed.createComponent(InstallPage)
    await fixture.whenStable()
    return fixture.nativeElement as HTMLElement
  }

  it('has one h1 and the three ways to install, each with its commands', async () => {
    const root = await render()

    expect(root.querySelectorAll('h1')).toHaveLength(1)
    expect(root.querySelector('h1')?.textContent).toBe('Installation')
    expect(root.querySelectorAll('app-install .install__panel')).toHaveLength(3)
    expect(root.textContent).toContain('docker compose up -d --build')
  })

  it('says what happens next and what is needed', async () => {
    const root = await render()

    expect(root.querySelectorAll('.install__steps li')).toHaveLength(3)
    expect(root.querySelector('.install__aside')?.textContent).toContain('JWT_SECRET')
  })
})
