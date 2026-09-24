import { CatalogSuggestion, OwnerRef } from './catalog.entity'

/** Public profile of an owner. Personal owners use the `@` form so a username can never collide with an app route. */
export function ownerLink(owner: OwnerRef): string[] {
  return owner.kind === 'personal' ? ['/@' + owner.slug] : ['/o', owner.slug]
}

export function repositoryLink(entry: CatalogSuggestion): string[] {
  return [...ownerLink(entry.owner), entry.repository.name]
}

/** Segments stay raw: the router encodes each one, which keeps a Docker name like `team/api` a single segment. */
export function packageLink(entry: CatalogSuggestion): string[] {
  return [...repositoryLink(entry), 'packages', entry.kind, entry.name]
}
