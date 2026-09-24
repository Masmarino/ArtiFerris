import {
  CatalogEntry,
  CatalogInfo,
  CatalogSearchResult,
  CatalogSuggestion,
  OwnerSummary,
} from '../domain/catalog.entity'

export function catalogEntry(overrides: Partial<CatalogEntry> = {}): CatalogEntry {
  return {
    kind: 'npm',
    name: 'hangar-demo',
    description: 'A small demo package used to try the catalog.',
    keywords: ['demo', 'catalog'],
    latest: '1.1.0',
    updated_at: '2026-09-24T11:20:00Z',
    downloads_7d: 0,
    match_kind: 'exact',
    repository: { name: 'test-npm' },
    owner: { kind: 'personal', slug: 'admin', display_name: 'admin' },
    registry_url: 'http://localhost:4200/npm/u/admin/test-npm/',
    image_reference: null,
    ...overrides,
  }
}

export function dockerEntry(overrides: Partial<CatalogEntry> = {}): CatalogEntry {
  return catalogEntry({
    kind: 'docker',
    name: 'team/api',
    description: null,
    keywords: [],
    latest: 'v2.0.1',
    repository: { name: 'images' },
    owner: { kind: 'organization', slug: 'acme', display_name: 'Acme Corp' },
    registry_url: null,
    image_reference: 'localhost:4200/o/acme/images/team/api',
    ...overrides,
  })
}

export function catalogSuggestion(overrides: Partial<CatalogSuggestion> = {}): CatalogSuggestion {
  return {
    kind: 'npm',
    name: 'hangar-demo',
    repository: { name: 'test-npm' },
    owner: { kind: 'personal', slug: 'admin', display_name: 'admin' },
    ...overrides,
  }
}

export function dockerSuggestion(overrides: Partial<CatalogSuggestion> = {}): CatalogSuggestion {
  return catalogSuggestion({
    kind: 'docker',
    name: 'team/api',
    repository: { name: 'images' },
    owner: { kind: 'organization', slug: 'acme', display_name: 'Acme Corp' },
    ...overrides,
  })
}

export function ownerSummary(overrides: Partial<OwnerSummary> = {}): OwnerSummary {
  return {
    kind: 'personal',
    slug: 'admin',
    display_name: 'admin',
    repository_count: 2,
    package_count: 3,
    image_count: 4,
    ...overrides,
  }
}

export function searchResult(
  items: CatalogEntry[],
  overrides: Partial<CatalogSearchResult> = {},
): CatalogSearchResult {
  return { items, total: items.length, page: 1, per_page: 20, ...overrides }
}

export const CATALOG_INFOS: CatalogInfo[] = [
  { format: 'npm', name: 'artiferris-npm', label: 'npm', entry_count: 12 },
  { format: 'docker', name: 'artiferris-docker', label: 'Docker', entry_count: 1 },
]
