import { CatalogSuggestion, OwnerRef } from './catalog.entity'

/** Personal owners use the `@` form so a username never collides with an app route. */
export function ownerLink(owner: OwnerRef): string[] {
  return owner.kind === 'personal' ? ['/@' + owner.slug] : ['/o', owner.slug]
}

export function repositoryLink(entry: CatalogSuggestion): string[] {
  return [...ownerLink(entry.owner), entry.repository.name]
}

/**
 * Segments stay raw: the router encodes them, so a Docker name like `team/api` stays one segment.
 */
export function packageLink(entry: CatalogSuggestion): string[] {
  return [...repositoryLink(entry), 'packages', entry.kind, entry.name]
}
