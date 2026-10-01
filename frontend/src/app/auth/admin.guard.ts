import { inject } from '@angular/core'
import { CanActivateFn, Router } from '@angular/router'
import { catchError, map, of } from 'rxjs'
import { MeService } from '../shell/application/me.service'

// Defense in depth: the backend enforces this too. Uses me.load() because a deep link can reach
// this guard first, with forceRefresh so a demotion is not missed.
export const adminGuard: CanActivateFn = () => {
  const me = inject(MeService)
  const router = inject(Router)

  return me.load({ forceRefresh: true }).pipe(
    map((response) => response.is_super_admin || router.createUrlTree(['/repositories'])),
    // Any failure, not just 401, fails closed.
    catchError(() => of(router.createUrlTree(['/repositories']))),
  )
}
