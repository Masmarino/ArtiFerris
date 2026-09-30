import { CanMatchFn } from '@angular/router'
import { CATALOGS } from './catalog/domain/catalog.registry'

/** Lets a route through only when the URL segment at `index` names a format we serve, so any other falls to the not-found page. */
export function knownFormatAt(index: number): CanMatchFn {
  return (_route, segments) => CATALOGS.some((catalog) => catalog.format === segments[index]?.path)
}
