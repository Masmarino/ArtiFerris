import { CatalogFormat } from './catalog.entity'

export const CATALOGS: readonly { format: CatalogFormat; name: string; label: string }[] = [
  { format: 'npm', name: 'artiferris-npm', label: 'npm' },
  { format: 'docker', name: 'artiferris-docker', label: 'Docker' },
]

export function catalogNameFromUrl(url: string): string | null {
  const segments = url.split(/[?#]/)[0].split('/')
  return segments.length === 2
    ? (CATALOGS.find((catalog) => catalog.name === segments[1])?.name ?? null)
    : null
}
