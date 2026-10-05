import { ActivatedRouteSnapshot } from '@angular/router'
import { pageTrail } from './page-trail'

function snapshot(
  data: Record<string, unknown>,
  child: ActivatedRouteSnapshot | null = null,
): ActivatedRouteSnapshot {
  return { data, firstChild: child } as unknown as ActivatedRouteSnapshot
}

describe('pageTrail', () => {
  it('reads the trail of the deepest route that declares one', () => {
    const leaf = snapshot({ trail: [{ labelKey: 'nav.users', link: '/users' }] })
    const root = snapshot({ trail: [{ labelKey: 'ignored', link: '/' }] }, snapshot({}, leaf))

    expect(pageTrail(root)).toEqual([{ labelKey: 'nav.users', link: '/users' }])
  })

  it('is empty for a page at the top of its rail', () => {
    expect(pageTrail(snapshot({}, snapshot({})))).toEqual([])
  })
})
