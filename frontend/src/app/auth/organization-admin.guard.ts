import { inject } from '@angular/core'
import { CanActivateFn, Router } from '@angular/router'
import { catchError, map, of } from 'rxjs'
import { MeService } from '../shell/application/me.service'

// Same as admin.guard.ts, with forceRefresh so a demotion is not missed.
export const organizationAdminGuard: CanActivateFn = (route) => {
  const me = inject(MeService)
  const router = inject(Router)
  const targetOrganizationId = route.paramMap.get('id')

  return me.load({ forceRefresh: true }).pipe(
    map(
      (response) =>
        response.is_super_admin ||
        (response.is_organization_admin && response.organization_id === targetOrganizationId) ||
        router.createUrlTree(['/repositories']),
    ),
    catchError(() => of(router.createUrlTree(['/repositories']))),
  )
}
