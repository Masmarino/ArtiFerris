/**
 * Tag for API paths: every interpolated value becomes one URL path segment. `.` and `..` are
 * refused, since browsers collapse them and the request would land on a different endpoint.
 */
export function apiPath(strings: TemplateStringsArray, ...segments: string[]): string {
  return strings.reduce((path, literal, index) => {
    if (index === 0) {
      return literal
    }
    const segment = String(segments[index - 1])
    if (segment === '' || segment === '.' || segment === '..') {
      throw new RangeError(`Invalid path segment: "${segment}"`)
    }
    return path + encodeURIComponent(segment) + literal
  }, '')
}
