import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { Component } from '@angular/core'
import { TestBed } from '@angular/core/testing'
import { PageTitleService } from '../../shell/page-title.service'
import { PageHeading } from './page-heading'

@Component({
  standalone: true,
  imports: [PageHeading],
  template: `<app-page-heading
    ><p header-meta class="intro">What the page holds</p></app-page-heading
  >`,
})
class Host {}

describe('PageHeading', () => {
  function render(title: string) {
    const fixture = TestBed.createComponent(Host)
    TestBed.inject(PageTitleService).title.set(title)
    fixture.detectChanges()
    return fixture.nativeElement as HTMLElement
  }

  it("opens the page on its h1, the page's current title", () => {
    const el = render('Dépôts')

    expect(el.querySelector('h1')?.textContent?.trim()).toBe('Dépôts')
    expect(el.querySelector('.gbt-page-header__meta .intro')?.textContent).toBe(
      'What the page holds',
    )
  })

  it('draws nothing until the page has a title', () => {
    const el = render('')

    expect(el.querySelector('gbt-page-header')).toBeNull()
  })
})

/**
 * The shell's bar names only what is above a page, so a page under the shell that forgot its heading
 * would have no visible title and no h1. Read from the routes' source: each page the shell loads.
 */
describe('the pages under the shell', () => {
  const app = join(process.cwd(), 'src', 'app')
  const routes = readFileSync(join(app, 'app.routes.ts'), 'utf8')
  // The shell's children: from its route to the public routes that follow it.
  const shell = routes.slice(
    routes.indexOf("import('./shell/app-shell')"),
    routes.indexOf('// After AppShell'),
  )
  const pages = [
    ...new Set(
      [...shell.matchAll(/loadComponent:\s*\(\)\s*=>\s*import\('\.\/([^']+)'\)/g)].map((m) => m[1]),
    ),
  ]

  it('are found', () => {
    expect(pages.length).toBeGreaterThan(8)
  })

  for (const page of pages) {
    it(`${page} opens on its own heading`, () => {
      const html = join(app, `${page}.html`)
      const template = existsSync(html)
        ? readFileSync(html, 'utf8')
        : readFileSync(join(app, `${page}.ts`), 'utf8')
      expect(template).toContain('<app-page-heading')
    })
  }
})
