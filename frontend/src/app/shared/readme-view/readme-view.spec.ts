import { TestBed } from '@angular/core/testing'
import { ReadmeView } from './readme-view'

function render(html: string | null) {
  const fixture = TestBed.createComponent(ReadmeView)
  fixture.componentRef.setInput('html', html)
  fixture.detectChanges()
  const el: HTMLElement = fixture.nativeElement
  return { fixture, el, content: el.querySelector('.readme-view__content') }
}

describe('ReadmeView', () => {
  it('is headed "README" and renders the HTML', () => {
    const { el, content } = render('<h2>Usage</h2><p>Call <code>go()</code></p>')

    expect(el.querySelector('h2.readme-view__title')!.textContent).toBe('README')
    expect(content!.querySelector('h2')!.textContent).toBe('Usage')
    expect(content!.querySelector('p code')!.textContent).toBe('go()')
    expect(el.querySelector('gbt-empty-state')).toBeNull()
  })

  it('keeps tables, code blocks and images', () => {
    const { content } = render(
      '<pre><code>npm i x</code></pre><table><tr><td>a</td></tr></table><img src="https://example.com/a.png" alt="logo">',
    )

    expect(content!.querySelector('pre code')!.textContent).toBe('npm i x')
    expect(content!.querySelector('table td')!.textContent).toBe('a')
    expect(content!.querySelector('img')!.getAttribute('src')).toBe('https://example.com/a.png')
  })

  it('keeps the target and rel the backend put on links', () => {
    const { content } = render(
      '<a href="https://example.com" target="_blank" rel="nofollow noopener noreferrer ugc">site</a>',
    )

    const link = content!.querySelector('a')!
    expect(link.getAttribute('target')).toBe('_blank')
    expect(link.getAttribute('rel')).toBe('nofollow noopener noreferrer ugc')
  })

  it('shows the empty state for null', () => {
    const { el, content } = render(null)

    expect(content).toBeNull()
    expect(el.querySelector('gbt-empty-state')!.textContent).toContain('Aucun README')
    expect(el.querySelector('h2.readme-view__title')!.textContent).toBe('README')
  })

  it.each(['', '   \n '])('shows the empty state for a blank string (%j)', (blank) => {
    const { el, content } = render(blank)

    expect(content).toBeNull()
    expect(el.textContent).toContain('Aucun README')
  })

  describe('as a second layer of defense', () => {
    it('drops script elements', () => {
      const { el } = render('<p>hi</p><script>window.__pwned = true</script>')

      expect(el.querySelector('script')).toBeNull()
      expect(el.textContent).not.toContain('__pwned')
      expect((window as unknown as { __pwned?: boolean }).__pwned).toBeUndefined()
    })

    it('drops event handler attributes', () => {
      const { el } = render(
        '<img src="x" onerror="window.__pwned = true"><a href="https://a.b" onclick="alert(1)">x</a>',
      )

      expect(el.querySelector('[onerror]')).toBeNull()
      expect(el.querySelector('[onclick]')).toBeNull()
      expect(el.querySelector('img')).not.toBeNull()
    })

    it('drops javascript: URLs', () => {
      const { el } = render('<a href="javascript:alert(1)">x</a>')

      expect(el.querySelector('a')!.getAttribute('href')).not.toMatch(/^javascript:/i)
    })
  })
})
