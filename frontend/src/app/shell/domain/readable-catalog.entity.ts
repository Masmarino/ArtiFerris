import {
  CatalogEntry,
  CatalogQuery,
  CatalogSearchResult,
} from '../../public/catalog/domain/catalog.entity'

export interface ReadableRepositoryRef {
  id: string
  name: string
  /** A proxy only holds what it has already cached from its upstream. */
  repo_type: 'hosted' | 'proxy'
}

export interface ReadableCatalogEntry extends CatalogEntry {
  repository: ReadableRepositoryRef
}

export interface ReadableCatalogSearchResult extends Omit<CatalogSearchResult, 'items'> {
  items: ReadableCatalogEntry[]
}

export type ReadableCatalogQuery = Omit<CatalogQuery, 'owner'>
