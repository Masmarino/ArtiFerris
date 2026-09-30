import { inject } from '@angular/core'
import { CanActivateFn, Router } from '@angular/router'
import { catchError, map, of } from 'rxjs'
import { MeService } from '../shell/application/me.service'

// As admin.guard.ts; organization admins pass too, since the backend scopes /api/users to their
// organization.
export const usersGuard: CanActivateFn = () => {
  const me = inject(MeService)
  const router = inject(Router)

  return me.load({ forceRefresh: true }).pipe(
    map(
      (response) =>
        response.is_super_admin ||
        response.is_organization_admin ||
        router.createUrlTree(['/repositories']),
    ),
    catchError(() => of(router.createUrlTree(['/repositories']))),
  )
}
