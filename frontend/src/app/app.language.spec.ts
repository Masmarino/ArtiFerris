import { Component, Injectable } from '@angular/core'
import { TestBed } from '@angular/core/testing'
import { Router, provideRouter } from '@angular/router'
import { Translation, TranslocoLoader, provideTransloco } from '@jsverse/transloco'
import { Observable, of } from 'rxjs'
import { App } from './app'
import { LanguageService } from './shared/i18n/language.service'

let pageCreations = 0
let shellChildCreations = 0

@Component({ selector: 'app-probe-page', template: '<p>page</p>' })
class ProbePage {
  constructor() {
    pageCreations++
  }
}

@Component({ selector: 'app-probe-child', template: '<p>child</p>' })
class ProbeChild {
  constructor() {
    shellChildCreations++
  }
}

@Injectable()
class EmptyLoader implements TranslocoLoader {
  getTranslation(): Observable<Translation> {
    return of({})
  }
}

describe('App on a language change', () => {
  beforeEach(() => {
    pageCreations = 0
    shellChildCreations = 0
    TestBed.configureTestingModule({
      providers: [
        provideTransloco({
          config: { availableLangs: ['fr', 'en'], defaultLang: 'fr', prodMode: false },
          loader: EmptyLoader,
        }),
        provideRouter([
          { path: 'public', component: ProbePage },
          {
            path: 'private',
            component: ProbeChild,
            data: { recreatesViewsOnLanguageChange: true },
          },
        ]),
      ],
    })
  })

  async function render(url: string) {
    const fixture = TestBed.createComponent(App)
    await TestBed.inject(Router).navigateByUrl(url)
    fixture.detectChanges()
    await fixture.whenStable()
    return fixture
  }

  it('re-creates the routed page so that it is rebuilt in the new language', async () => {
    const fixture = await render('/public')
    expect(pageCreations).toBe(1)

    await TestBed.inject(LanguageService).use('en')
    fixture.detectChanges()
    await fixture.whenStable()

    expect(pageCreations).toBe(2)
    expect(fixture.nativeElement.textContent).toContain('page')
  })

  it('leaves to the route the re-creation of its own views when it asks to', async () => {
    const fixture = await render('/private')
    expect(shellChildCreations).toBe(1)

    await TestBed.inject(LanguageService).use('en')
    fixture.detectChanges()
    await fixture.whenStable()

    expect(shellChildCreations).toBe(1)
  })
})
