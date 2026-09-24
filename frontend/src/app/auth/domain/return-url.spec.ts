import { safeReturnUrl } from './return-url'

describe('safeReturnUrl', () => {
  it.each(['/repositories', '/repositories/abc?tab=x#top', '/o/acme/images', '/@alice'])(
    'keeps the same-origin path %j',
    (url) => {
      expect(safeReturnUrl(url)).toBe(url)
    },
  )

  it.each([
    null,
    undefined,
    42,
    '',
    'repositories',
    '//evil.example',
    '//evil.example/x',
    'https://evil.example',
    'http://localhost/x',
    'javascript:alert(1)',
    'data:text/html,x',
    '/\\evil.example',
    '/\t/evil.example',
    '/\n/evil.example',
    '/login',
    '/login?returnUrl=/x',
  ])('rejects %j', (url) => {
    expect(safeReturnUrl(url)).toBeNull()
  })
})
