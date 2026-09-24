import { UrlSegment, UrlSegmentGroup } from '@angular/router'
import { personalOwnerMatcher } from './owner-url-matcher'

function match(...paths: string[]) {
  const segments = paths.map((path) => new UrlSegment(path, {}))
  return personalOwnerMatcher(segments, new UrlSegmentGroup(segments, {}), {} as never)
}

describe('personalOwnerMatcher', () => {
  it('matches @username and exposes the username without the @', () => {
    const result = match('@alice')

    expect(result?.consumed.map((s) => s.path)).toEqual(['@alice'])
    expect(result?.posParams?.['username'].path).toBe('alice')
  })

  it('keeps the rest of the name untouched', () => {
    expect(match('@a@b')?.posParams?.['username'].path).toBe('a@b')
  })

  it('does not match a segment without the @', () => {
    expect(match('alice')).toBeNull()
    expect(match('explorer')).toBeNull()
  })

  it('does not match a lone @', () => {
    expect(match('@')).toBeNull()
  })

  it('does not match a longer path: the repository routes handle those', () => {
    expect(match('@alice', 'my-lib')).toBeNull()
  })

  it('does not match an empty path', () => {
    expect(match()).toBeNull()
  })
})
