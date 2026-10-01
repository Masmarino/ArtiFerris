const REVOKE_DELAY_MS = 1000

/** Saves content fetched through HttpClient, so auth headers go along. */
export function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = url
  link.download = filename
  link.click()
  // Some browsers read the blob after click() returns.
  setTimeout(() => URL.revokeObjectURL(url), REVOKE_DELAY_MS)
}
