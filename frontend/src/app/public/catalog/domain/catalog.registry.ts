import { CatalogFormat } from './catalog.entity'

/** Mirrors the backend format registry: one public catalog per supported format. */
export const CATALOGS: readonly { format: CatalogFormat; name: string; label: string }[] = [
  { format: 'npm', name: 'artiferris-npm', label: 'npm' },
  { format: 'docker', name: 'artiferris-docker', label: 'Docker' },
]

/** The catalog name when the URL is a catalog page, otherwise null. */
export function catalogNameFromUrl(url: string): string | null {
  const segments = url.split(/[?#]/)[0].split('/')
  return segments.length === 2
    ? (CATALOGS.find((catalog) => catalog.name === segments[1])?.name ?? null)
    : null
}
