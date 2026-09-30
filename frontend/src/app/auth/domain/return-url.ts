const PLACEHOLDER_ORIGIN = 'http://placeholder.invalid'

export function safeReturnUrl(candidate: unknown): string | null {
  if (typeof candidate !== 'string' || !candidate.startsWith('/') || candidate.startsWith('//')) {
    return null
  }
  // Browsers drop tabs and newlines and read backslashes as slashes: '/\t/host' becomes '//host'.
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f\\]/.test(candidate)) {
    return null
  }
  let url: URL
  try {
    url = new URL(candidate, PLACEHOLDER_ORIGIN)
  } catch {
    return null
  }
  if (url.origin !== PLACEHOLDER_ORIGIN || url.pathname === '/login') {
    return null
  }
  return candidate
}
