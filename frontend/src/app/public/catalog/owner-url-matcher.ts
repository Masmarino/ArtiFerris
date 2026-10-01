import { UrlMatcher, UrlSegment } from '@angular/router'

export const personalOwnerMatcher: UrlMatcher = (segments) => {
  const [segment] = segments
  if (segments.length !== 1 || !segment.path.startsWith('@') || segment.path.length < 2) {
    return null
  }
  return { consumed: segments, posParams: { username: new UrlSegment(segment.path.slice(1), {}) } }
}
