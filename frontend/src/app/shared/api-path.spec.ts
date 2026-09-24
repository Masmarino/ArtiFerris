import { apiPath } from './api-path'

describe('apiPath', () => {
  it('leaves the fixed parts alone and encodes each value as one segment', () => {
    expect(apiPath`/api/repositories/${'repo-1'}/packages/npm/${'@scope/pkg'}`).toBe(
      '/api/repositories/repo-1/packages/npm/%40scope%2Fpkg',
    )
  })

  it('cannot be made to leave its path with a slash or an encoded slash', () => {
    expect(apiPath`/api/repositories/${'a/../../users'}`).toBe(
      '/api/repositories/a%2F..%2F..%2Fusers',
    )
    expect(apiPath`/api/repositories/${'a%2F..'}`).toBe('/api/repositories/a%252F..')
  })

  it.each(['..', '.', ''])('refuses the segment %j', (segment) => {
    expect(() => apiPath`/api/repositories/${segment}`).toThrow(RangeError)
  })

  it('encodes query-like characters', () => {
    expect(apiPath`/api/users/${'a?b#c'}`).toBe('/api/users/a%3Fb%23c')
  })
})
