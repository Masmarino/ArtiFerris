import { ActivatedRouteSnapshot } from '@angular/router'

/**
 * One step above a page in the bar, declared on its route with a translation key and the page it leads to. A step
 * with no page of its own (a section's name above its first page) has no link.
 */
export interface TrailStep {
  labelKey: string
  link?: string
}

/**
 * What sits above the current page, declared on its route as `data: { trail: [...] }`: the bar shows it before the
 * page's title. The deepest route that declares one wins; a page with none is at the top of its rail.
 */
export function pageTrail(root: ActivatedRouteSnapshot): TrailStep[] {
  let trail: TrailStep[] = []
  for (let route: ActivatedRouteSnapshot | null = root; route; route = route.firstChild) {
    const declared = route.data['trail'] as TrailStep[] | undefined
    if (declared) trail = declared
  }
  return trail
}

export const ADMIN_TRAIL: TrailStep[] = [{ labelKey: 'nav.administration', link: '/admin' }]
/** Above the administration's own page, which is its dashboard: the section's name, as in FerrisGit. */
export const ADMINISTRATION_TRAIL: TrailStep[] = [{ labelKey: 'nav.administration' }]
