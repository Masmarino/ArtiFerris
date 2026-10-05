import { TestBed } from '@angular/core/testing'
import { provideRouter } from '@angular/router'
import { AuthService } from '../auth/application/auth.service'
import { NO_SUGGESTIONS } from '../public/catalog/testing/no-suggestions'
import { PageTitleService } from '../shell/page-title.service'
import { NotFoundPage } from './not-found-page'

describe('NotFoundPage', () => {
  function render(signedIn = false) {
    TestBed.configureTestingModule({
      providers: [
        provideRouter([]),
        NO_SUGGESTIONS,
        { provide: AuthService, useValue: { isAuthenticated: () => signedIn } },
      ],
    })
    const fixture = TestBed.createComponent(NotFoundPage)
    fixture.detectChanges()
    return fixture
  }

  it('says the page does not exist and links home', () => {
    const el: HTMLElement = render().nativeElement

    expect(el.querySelector('h1')?.textContent?.trim()).toBe('Page introuvable')
    expect(el.querySelector('main a[href="/"]')?.textContent).toContain("Retour à l'accueil")
  })

  it('offers a visitor to sign in and come back to this address', () => {
    const el: HTMLElement = render().nativeElement

    const signIn = el.querySelector<HTMLAnchorElement>('main a[href^="/login"]')
    expect(signIn?.textContent).toContain('Se connecter')
    expect(signIn?.getAttribute('href')).toContain('returnUrl=')
  })

  it('only links home once signed in', () => {
    const el: HTMLElement = render(true).nativeElement

    expect(el.querySelector('main a[href^="/login"]')).toBeNull()
    expect(el.querySelector('main a[href="/"]')).not.toBeNull()
  })

  it('sets the page title', () => {
    render()

    expect(TestBed.inject(PageTitleService).title()).toBe('Page introuvable')
  })
})
