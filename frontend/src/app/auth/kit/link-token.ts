import { afterNextRender, inject } from '@angular/core'
import { ActivatedRoute, Router } from '@angular/router'
import { activationToken } from '@masmarino/gabarit/auth'

/**
 * Reads the token of a mailed link (`#token=…`, or `?token=` for older mails) and scrubs it, and any
 * other parameter, from the address bar and history. Call it from a field initializer (it needs an
 * injection context) and keep the result in a plain field: the kit's page restarts when its `token`
 * changes, so it cannot follow the URL. A malformed token comes back as `null`.
 */
export function consumeLinkToken(path: string): string | null {
  const router = inject(Router)
  const { fragment, queryParamMap } = inject(ActivatedRoute).snapshot
  if (fragment !== null || queryParamMap.keys.length > 0) {
    afterNextRender(
      () => void router.navigateByUrl(path, { replaceUrl: true }).catch(() => undefined),
    )
  }
  return activationToken(fragment, queryParamMap.get('token'))
}
