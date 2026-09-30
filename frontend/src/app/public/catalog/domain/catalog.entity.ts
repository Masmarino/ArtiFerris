export type CatalogFormat = 'npm' | 'docker'
export type CatalogSort = 'relevance' | 'updated' | 'popular'
export type CatalogMatchKind = 'exact' | 'prefix' | 'contains' | 'text' | 'fuzzy'

export interface CatalogInfo {
  format: CatalogFormat
  name: string
  label: string
  entry_count: number
}

export type OwnerKind = 'personal' | 'organization'

export interface OwnerRef {
  kind: OwnerKind
  slug: string
}

export interface CatalogOwner extends OwnerRef {
  display_name: string
}

export interface OwnerSummary extends CatalogOwner {
  repository_count: number
  package_count: number
  image_count: number
}

export interface CatalogSuggestion {
  kind: CatalogFormat
  name: string
  repository: { name: string }
  owner: CatalogOwner
}

export interface CatalogEntry extends CatalogSuggestion {
  description: string | null
  keywords: string[]
  /** Latest version (npm) or latest tag (docker). */
  latest: string | null
  updated_at: string
  downloads_7d: number
  /** `null` without search text. `fuzzy` is a typo match, always last. */
  match_kind: CatalogMatchKind | null
  registry_url: string | null
  image_reference: string | null
}

export interface CatalogSearchResult {
  items: CatalogEntry[]
  total: number
  page: number
  per_page: number
}

export interface SuggestOptions {
  format?: CatalogFormat | null
  owner?: OwnerRef | null
  limit?: number
}

export interface CatalogQuery {
  q?: string
  format?: CatalogFormat
  owner?: OwnerRef
  sort?: CatalogSort
  page?: number
  perPage?: number
}
