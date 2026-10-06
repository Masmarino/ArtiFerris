import { inlineCodeToHtml } from './inline-code.pipe'

describe('inlineCodeToHtml', () => {
  it('turns backtick spans into code elements', () => {
    expect(inlineCodeToHtml('Use `needs` here')).toBe('Use <code>needs</code> here')
  })

  it('escapes everything else, so a translation can never inject markup', () => {
    expect(inlineCodeToHtml('<img src=x onerror=alert(1)> & `a<b`')).toBe(
      '&lt;img src=x onerror=alert(1)&gt; &amp; <code>a&lt;b</code>',
    )
  })
})
