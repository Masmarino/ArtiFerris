import { CatalogFormat, CatalogSort } from './catalog.entity'
import { CATALOGS } from './catalog.registry'

export function parseFormat(value: string | null): CatalogFormat | null {
  return CATALOGS.find((catalog) => catalog.format === value)?.format ?? null
}

export function parseSort(value: string | null): CatalogSort | null {
  return value === 'relevance' || value === 'updated' || value === 'popular' ? value : null
}

// The API takes the page as a u32 and rejects anything else.
const MAX_PAGE = 4_294_967_295

export function parsePage(value: string | null): number {
  if (value === null || !/^[1-9]\d{0,9}$/.test(value)) {
    return 1
  }
  const page = Number(value)
  return page <= MAX_PAGE ? page : 1
}
