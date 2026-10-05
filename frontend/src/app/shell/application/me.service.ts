import { Injectable, effect, inject, signal } from '@angular/core'
import { Observable, catchError, map, shareReplay, tap, throwError } from 'rxjs'
import { AuthService } from '../../auth/application/auth.service'
import { SessionToken } from '../../auth/application/session-token'
import { MeResponse } from '../domain/me.entity'
import { LanguageService } from '../../shared/i18n/language.service'
import { detectBrowserLanguage, isSupported } from '../../shared/i18n/languages'
import { ME_PORT } from './me.port'

@Injectable({ providedIn: 'root' })
export class MeService {
  private readonly port = inject(ME_PORT)
  private readonly auth = inject(AuthService)
  private readonly languageService = inject(LanguageService)
  private readonly session = inject(SessionToken)

  readonly username = signal<string | null>(null)
  readonly isSuperAdmin = signal(false)
  readonly createdAt = signal<string | null>(null)
  readonly organizationId = signal<string | null>(null)
  readonly isOrganizationAdmin = signal(false)
  readonly language = signal<string | null>(null)
  /** The address ArtiFerris writes to, once verified; `null` when there is none. */
  readonly email = signal<string | null>(null)

  private cached$: Observable<MeResponse> | null = null
  /** A token this account was handed for itself (a password change): not another account signing in. */
  private renewedToken: string | null = null

  constructor() {
    // Skip the first run, or it would wipe state set on inject.
    let previousToken = this.auth.token()
    effect(() => {
      const token = this.auth.token()
      if (token === previousToken) {
        return
      }
      previousToken = token
      if (token !== null && token === this.renewedToken) {
        this.renewedToken = null
        return
      }
      this.cached$ = null
      this.username.set(null)
      this.isSuperAdmin.set(false)
      this.createdAt.set(null)
      this.organizationId.set(null)
      this.isOrganizationAdmin.set(false)
      this.language.set(null)
      this.email.set(null)
      // Signing out, or another account signing in: back to the browser's language.
      this.showBrowserLanguage()
    })
  }

  /** Cached per token: callers share one request. */
  load(options?: { forceRefresh?: boolean }): Observable<MeResponse> {
    if (!this.cached$ || options?.forceRefresh) {
      this.cached$ = this.port.load().pipe(
        tap((me) => {
          this.username.set(me.username)
          this.isSuperAdmin.set(me.is_super_admin)
          this.createdAt.set(me.created_at)
          this.organizationId.set(me.organization_id)
          this.isOrganizationAdmin.set(me.is_organization_admin)
          this.language.set(me.language ?? null)
          this.email.set(me.email ?? null)
          this.applyAccountLanguage(me.language)
        }),
        // Never cache a failure.
        catchError((err: unknown) => {
          this.cached$ = null
          return throwError(() => err)
        }),
        shareReplay(1),
      )
    }
    return this.cached$
  }

  /**
   * The account's language wins. An account without one (`null`; `undefined` is an older server)
   * takes the browser's and keeps it by recording it.
   */
  private applyAccountLanguage(saved: string | null | undefined): void {
    if (saved && isSupported(saved)) {
      if (saved !== this.languageService.language()) {
        void this.languageService.use(saved)
      }
      return
    }
    if (saved !== null) {
      return
    }
    const browser = detectBrowserLanguage()
    this.showBrowserLanguage()
    this.port.setLanguage(browser).subscribe({
      next: () => this.language.set(browser),
      // Best effort: the next sign-in tries again.
      error: () => undefined,
    })
  }

  private showBrowserLanguage(): void {
    const browser = detectBrowserLanguage()
    if (browser !== this.languageService.language()) {
      void this.languageService.use(browser)
    }
  }

  /**
   * The change ends every session, this one included; the server hands back a fresh session, kept
   * here, so the caller stays signed in while the other sessions stay ended.
   */
  changePassword(currentPassword: string, newPassword: string): Observable<void> {
    return this.port.changePassword(currentPassword, newPassword).pipe(
      tap(({ token }) => {
        this.renewedToken = token
        this.session.set(token)
      }),
      map(() => undefined),
    )
  }

  setLanguage(language: string): Observable<void> {
    return this.port.setLanguage(language).pipe(tap(() => this.language.set(language)))
  }
}
