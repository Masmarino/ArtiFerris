import { DestroyRef, Injectable, effect, inject, signal, untracked } from '@angular/core'
import { NavigationStart, Router } from '@angular/router'
import { filter } from 'rxjs'
import { SessionToken } from '../auth/application/session-token'

export interface ConfirmOptions {
  heading: string
  message: string
  confirmLabel?: string
  cancelLabel?: string
  danger?: boolean
  /** The exact text the user has to retype; switches to Gabarit's type-to-confirm dialog. */
  typeToConfirm?: string
}

export interface PendingConfirm {
  options: ConfirmOptions
  resolve: (confirmed: boolean) => void
}

/** App-wide confirmation dialog — `ConfirmHost` renders the pending request, this owns its state. */
@Injectable({ providedIn: 'root' })
export class ConfirmService {
  readonly pending = signal<PendingConfirm | null>(null)

  constructor() {
    // A dialog outliving its page (Back button, 401, sign-out) would act on a stale target.
    const subscription = inject(Router)
      .events.pipe(filter((event) => event instanceof NavigationStart))
      .subscribe(() => this.answer(false))
    inject(DestroyRef).onDestroy(() => subscription.unsubscribe())

    const session = inject(SessionToken)
    let previousToken = session.value()
    effect(() => {
      const token = session.value()
      if (token !== previousToken) {
        previousToken = token
        this.answer(false)
      }
    })
  }

  ask(options: ConfirmOptions): Promise<boolean> {
    // Untracked: a caller running inside an effect must not become dependent on `pending`.
    untracked(() => this.pending())?.resolve(false)
    return new Promise((resolve) => this.pending.set({ options, resolve }))
  }

  answer(confirmed: boolean): void {
    const pending = this.pending()
    this.pending.set(null)
    pending?.resolve(confirmed)
  }
}
