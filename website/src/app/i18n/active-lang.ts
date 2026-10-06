import { Signal, inject } from '@angular/core'
import { toSignal } from '@angular/core/rxjs-interop'
import { TranslocoService } from '@jsverse/transloco'
import { Lang } from './languages'

/** The language the page is currently shown in. Call it in an injection context. */
export function injectActiveLang(): Signal<Lang> {
  const transloco = inject(TranslocoService)
  return toSignal(transloco.langChanges$, {
    initialValue: transloco.getActiveLang(),
  }) as Signal<Lang>
}
