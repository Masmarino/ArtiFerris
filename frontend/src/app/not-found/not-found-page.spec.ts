import { TestBed } from '@angular/core/testing'
import { provideRouter } from '@angular/router'
import { AuthService } from '../auth/application/auth.service'
import { NO_SUGGESTIONS } from '../public/catalog/testing/no-suggestions'
import { PageTitleService } from '../shell/page-title.service'
import { NotFoundPage } from './not-found-page'

describe('NotFoundPage', () => {
  function render() {
    TestBed.configureTestingModule({
      providers: [
        provideRouter([]),
        NO_SUGGESTIONS,
        { provide: AuthService, useValue: { isAuthenticated: () => false } },
      ],
    })
    const fixture = TestBed.createComponent(NotFoundPage)
    fixture.detectChanges()
    return fixture
  }

  it('says the page does not exist and links home', () => {
    const el: HTMLElement = render().nativeElement

    expect(el.querySelector('h1')?.textContent).toBe('Page introuvable')
    expect(el.querySelector('main a[href="/"]')?.textContent).toContain("Retour à l'accueil")
  })

  it('sets the page title', () => {
    render()

    expect(TestBed.inject(PageTitleService).title()).toBe('Page introuvable')
  })
})
