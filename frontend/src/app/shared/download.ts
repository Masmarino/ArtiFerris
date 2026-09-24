const REVOKE_DELAY_MS = 1000

/** Saves in-memory content fetched via HttpClient (so auth headers attach), not a plain `<a href>`. */
export function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = url
  link.download = filename
  link.click()
  // Some browsers start reading the blob after click() returns.
  setTimeout(() => URL.revokeObjectURL(url), REVOKE_DELAY_MS)
}
