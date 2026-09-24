import { catalogEntry, dockerEntry } from '../../public/catalog/testing/catalog.fixtures'
import {
  ReadableCatalogEntry,
  ReadableCatalogSearchResult,
  ReadableRepositoryRef,
} from '../domain/readable-catalog.entity'

const HOSTED: ReadableRepositoryRef = { id: 'r-npm', name: 'test-npm', repo_type: 'hosted' }

export function readableEntry(overrides: Partial<ReadableCatalogEntry> = {}): ReadableCatalogEntry {
  return { ...catalogEntry({ name: 'left-pad' }), repository: HOSTED, ...overrides }
}

export function readableDockerEntry(
  overrides: Partial<ReadableCatalogEntry> = {},
): ReadableCatalogEntry {
  return {
    ...dockerEntry({ name: 'api' }),
    repository: { id: 'r-docker', name: 'images', repo_type: 'hosted' },
    ...overrides,
  }
}

export function proxiedEntry(overrides: Partial<ReadableCatalogEntry> = {}): ReadableCatalogEntry {
  return readableEntry({
    name: 'lodash',
    repository: { id: 'r-proxy', name: 'npmjs-proxy', repo_type: 'proxy' },
    ...overrides,
  })
}

export function readableSearchResult(
  items: ReadableCatalogEntry[],
  overrides: Partial<ReadableCatalogSearchResult> = {},
): ReadableCatalogSearchResult {
  return { items, total: items.length, page: 1, per_page: 5, ...overrides }
}
