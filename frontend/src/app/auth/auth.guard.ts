import { inject } from '@angular/core'
import { CanActivateFn, Router } from '@angular/router'
import { AuthService } from './application/auth.service'
import { safeReturnUrl } from './domain/return-url'

export const authGuard: CanActivateFn = (_route, state) => {
  const auth = inject(AuthService)
  const router = inject(Router)

  if (auth.isAuthenticated()) {
    return true
  }
  const returnUrl = safeReturnUrl(state.url)
  return router.createUrlTree(
    ['/login'],
    returnUrl && returnUrl !== '/' ? { queryParams: { returnUrl } } : {},
  )
}
