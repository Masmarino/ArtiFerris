import { CanMatchFn } from '@angular/router'
import { CATALOGS } from './catalog/domain/catalog.registry'

/**
 * Lets a route through only when the segment at `index` is a format we serve; anything else falls
 * to not-found.
 */
export function knownFormatAt(index: number): CanMatchFn {
  return (_route, segments) => CATALOGS.some((catalog) => catalog.format === segments[index]?.path)
}
