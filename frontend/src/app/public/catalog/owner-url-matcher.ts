import { UrlMatcher, UrlSegment } from '@angular/router'

/** Matches exactly one segment of the form `@username`, exposing it as `username` without the `@`. */
export const personalOwnerMatcher: UrlMatcher = (segments) => {
  const [segment] = segments
  if (segments.length !== 1 || !segment.path.startsWith('@') || segment.path.length < 2) {
    return null
  }
  return { consumed: segments, posParams: { username: new UrlSegment(segment.path.slice(1), {}) } }
}
