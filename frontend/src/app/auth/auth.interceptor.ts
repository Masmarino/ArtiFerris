import { inject } from '@angular/core'
import { HttpErrorResponse, HttpInterceptorFn } from '@angular/common/http'
import { Router } from '@angular/router'
import { catchError, throwError } from 'rxjs'
import { AuthService } from './application/auth.service'
import { safeReturnUrl } from './domain/return-url'

// Endpoints used before a session exists: their 401 is not a dead session. Not a `/api/auth/*`
// prefix, since logout is session-authenticated.
const UNAUTHENTICATED_AUTH_ENDPOINT =
  /\/api\/auth\/(?:login|register|activate|sso\/ldap|mfa\/verify|mfa\/passkey\/(?:start|finish)|mfa\/setup\/[\w/-]+)$/

// Only relative same-origin URLs get the token, never an absolute or protocol-relative one (it
// would leak to a third party).
function isRelativeUrl(url: string): boolean {
  return url.startsWith('/') && !url.startsWith('//')
}

export const authInterceptor: HttpInterceptorFn = (req, next) => {
  // Injected here because the catchError callback runs outside the injection context.
  const auth = inject(AuthService)
  const router = inject(Router)

  const token = auth.token()
  const authorizedReq =
    token && isRelativeUrl(req.url)
      ? req.clone({ setHeaders: { Authorization: `Bearer ${token}` } })
      : req

  return next(authorizedReq).pipe(
    catchError((error: unknown) => {
      // A 401 means a dead session only when a token was sent. An anonymous request can 401 for its
      // own reasons and must not redirect a visitor.
      if (
        error instanceof HttpErrorResponse &&
        error.status === 401 &&
        authorizedReq !== req &&
        !UNAUTHENTICATED_AUTH_ENDPOINT.test(req.url.split('?')[0])
      ) {
        auth.logout()
        redirectToLogin(router)
      }
      return throwError(() => error)
    }),
  )
}

// Remember the page so login can return to it; a later 401 must not overwrite it.
function redirectToLogin(router: Router): void {
  if (router.url.split(/[?#]/)[0] === '/login') {
    return
  }
  const returnUrl = safeReturnUrl(router.url)
  const target = router.createUrlTree(
    ['/login'],
    returnUrl && returnUrl !== '/' ? { queryParams: { returnUrl } } : {},
  )
  void router.navigateByUrl(router.serializeUrl(target))
}
