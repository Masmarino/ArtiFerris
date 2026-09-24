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

/** What the search-as-you-type popup shows: a name and where to find it. */
export interface CatalogSuggestion {
  kind: CatalogFormat
  name: string
  repository: { name: string }
  owner: CatalogOwner
}

export interface CatalogEntry extends CatalogSuggestion {
  description: string | null
  keywords: string[]
  /** Latest version (npm) or most recently updated tag (docker). */
  latest: string | null
  updated_at: string
  /** Downloads over the last 7 days, indicative only. */
  downloads_7d: number
  /** `null` when the search had no text. `fuzzy` is a typo-tolerant name match, always ranked last. */
  match_kind: CatalogMatchKind | null
  /** npm entries only. */
  registry_url: string | null
  /** Docker entries only, without tag or scheme. */
  image_reference: string | null
}

export interface CatalogSearchResult {
  items: CatalogEntry[]
  total: number
  page: number
  per_page: number
}

/** What narrows the suggestions on the server. `limit` defaults to 8 there and is capped at 20. */
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
