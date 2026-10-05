import { DestroyRef, Injector, inject } from '@angular/core'
import { takeUntilDestroyed } from '@angular/core/rxjs-interop'
import { ActivatedRoute, Router } from '@angular/router'

/** The query parameter the quick search sets to open a page's creation dialog, e.g. `?new=repository`. */
export const NEW_PARAM = 'new'

/**
 * Opens a page's creation dialog when the URL asks for it (`?new=<kind>`), then drops the parameter so a reload or the
 * Back button doesn't open it again. Also works when the page is already shown. Without an injector it needs an
 * injection context; pass one to call it from `ngOnInit`, once the inputs that pick the kind are set.
 */
export function openWhenAsked(
  kind: string,
  open: () => void,
  injector: Injector = inject(Injector),
): void {
  const route = injector.get(ActivatedRoute)
  const router = injector.get(Router)
  route.queryParamMap.pipe(takeUntilDestroyed(injector.get(DestroyRef))).subscribe((params) => {
    if (params.get(NEW_PARAM) !== kind) {
      return
    }
    open()
    void router.navigate([], {
      relativeTo: route,
      queryParams: { [NEW_PARAM]: null },
      queryParamsHandling: 'merge',
      replaceUrl: true,
    })
  })
}
