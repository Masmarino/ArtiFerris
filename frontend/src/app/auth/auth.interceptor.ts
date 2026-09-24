import { inject } from '@angular/core'
import { HttpErrorResponse, HttpInterceptorFn } from '@angular/common/http'
import { Router } from '@angular/router'
import { catchError, throwError } from 'rxjs'
import { AuthService } from './application/auth.service'
import { safeReturnUrl } from './domain/return-url'

// Endpoints reachable before a session exists: their 401 (wrong password, bad or expired MFA
// token, failed bind) is not "your session died". Not a `/api/auth/*` prefix match, since
// logout and logout-all are session-authenticated.
const UNAUTHENTICATED_AUTH_ENDPOINT =
  /\/api\/auth\/(?:login|register|activate|sso\/ldap|mfa\/verify|mfa\/passkey\/(?:start|finish)|mfa\/setup\/[\w/-]+)$/

// The app only ever calls relative, same-origin URLs (e.g. '/api/...'). Only attach the
// token to those — never to an absolute URL (a CDN, a telemetry endpoint), where it would
// leak to a third party. A protocol-relative URL ('//host/...') is absolute too, so it's
// excluded alongside anything with a scheme.
function isRelativeUrl(url: string): boolean {
  return url.startsWith('/') && !url.startsWith('//')
}

export const authInterceptor: HttpInterceptorFn = (req, next) => {
  // Both must be injected here, synchronously: the catchError callback below runs
  // later, outside the injection context.
  const auth = inject(AuthService)
  const router = inject(Router)

  const token = auth.token()
  const authorizedReq =
    token && isRelativeUrl(req.url)
      ? req.clone({ setHeaders: { Authorization: `Bearer ${token}` } })
      : req

  return next(authorizedReq).pipe(
    catchError((error: unknown) => {
      // A 401 only means "your session died" when a session's token was actually sent. An
      // anonymous request (no token attached) can still 401 for its own business reasons — e.g.
      // a public repository's npm-advisory audit, which anonymous callers may only read from
      // cache, not trigger — and that must not log out or redirect a visitor who was never
      // signed in to begin with.
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

// Keeps the page the user was on so login can bring them back. A second 401 landing after
// the first redirect finished (router already on /login) must not overwrite it.
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
