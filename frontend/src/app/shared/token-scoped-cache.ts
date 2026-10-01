import { effect, inject } from '@angular/core'
import { Observable, catchError, shareReplay, throwError } from 'rxjs'
import { SessionToken } from '../auth/application/session-token'

export class TokenScopedCache<T> {
  private readonly session = inject(SessionToken)
  private entry: { token: string | null; value$: Observable<T> } | null = null

  constructor(private readonly load: () => Observable<T>) {
    let previousToken = this.session.value()
    effect(() => {
      const token = this.session.value()
      if (token !== previousToken) {
        previousToken = token
        this.clear()
      }
    })
  }

  get(forceRefresh = false): Observable<T> {
    const token = this.session.value()
    if (!this.entry || this.entry.token !== token || forceRefresh) {
      const entry = { token, value$: null as unknown as Observable<T> }
      entry.value$ = this.load().pipe(
        catchError((err: unknown) => {
          if (this.entry === entry) {
            this.entry = null
          }
          return throwError(() => err)
        }),
        shareReplay(1),
      )
      this.entry = entry
    }
    return this.entry.value$
  }

  clear(): void {
    this.entry = null
  }
}
