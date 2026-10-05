import { Component, Injector, OnInit, Type, inject } from '@angular/core'
import { TestBed } from '@angular/core/testing'
import { Router, provideRouter } from '@angular/router'
import { RouterTestingHarness } from '@angular/router/testing'
import { openWhenAsked } from './open-when-asked'

@Component({ selector: 'app-test-page', template: '' })
class PageWithDialog {
  opened = 0
  constructor() {
    openWhenAsked('repository', () => this.opened++)
  }
}

@Component({ selector: 'app-test-late-page', template: '' })
class PageDecidingLate implements OnInit {
  private readonly injector = inject(Injector)
  opened = 0
  ngOnInit(): void {
    openWhenAsked('project', () => this.opened++, this.injector)
  }
}

describe('openWhenAsked', () => {
  async function setup<T>(url: string, component: Type<T>) {
    TestBed.configureTestingModule({
      providers: [
        provideRouter([
          { path: 'repositories', component: PageWithDialog },
          { path: 'my-repository', component: PageDecidingLate },
        ]),
      ],
    })
    const harness = await RouterTestingHarness.create()
    const page = await harness.navigateByUrl(url, component)
    await harness.fixture.whenStable()
    return { harness, page, router: TestBed.inject(Router) }
  }

  it('opens the dialog the URL asks for, then drops the parameter so a reload does not reopen it', async () => {
    const { page, router } = await setup('/repositories?new=repository&sort=name', PageWithDialog)

    expect(page.opened).toBe(1)
    expect(router.url).toBe('/repositories?sort=name')
  })

  it('leaves the page alone with another kind', async () => {
    const { page, router } = await setup('/repositories?new=user', PageWithDialog)

    expect(page.opened).toBe(0)
    expect(router.url).toBe('/repositories?new=user')
  })

  it('opens it again when asked on the page already shown', async () => {
    const { harness, page, router } = await setup('/repositories', PageWithDialog)

    await harness.navigateByUrl('/repositories?new=repository')
    await harness.fixture.whenStable()

    expect(page.opened).toBe(1)
    expect(router.url).toBe('/repositories')
  })

  it('works from ngOnInit with the injector given', async () => {
    const { page, router } = await setup('/my-repository?new=project', PageDecidingLate)

    expect(page.opened).toBe(1)
    expect(router.url).toBe('/my-repository')
  })
})
