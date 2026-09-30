import { Injectable, effect, inject, signal } from '@angular/core'
import { Observable, catchError, shareReplay, tap, throwError } from 'rxjs'
import { AuthService } from '../../auth/application/auth.service'
import { MeResponse } from '../domain/me.entity'
import { LanguageService } from '../../shared/i18n/language.service'
import { detectBrowserLanguage, isSupported } from '../../shared/i18n/languages'
import { ME_PORT } from './me.port'

@Injectable({ providedIn: 'root' })
export class MeService {
  private readonly port = inject(ME_PORT)
  private readonly auth = inject(AuthService)
  private readonly languageService = inject(LanguageService)

  readonly username = signal<string | null>(null)
  readonly isSuperAdmin = signal(false)
  readonly createdAt = signal<string | null>(null)
  readonly organizationId = signal<string | null>(null)
  readonly isOrganizationAdmin = signal(false)
  /** The language the user chose; `null` until they have. */
  readonly language = signal<string | null>(null)

  private cached$: Observable<MeResponse> | null = null

  constructor() {
    // Skip the effect's first run, or we'd wipe out state a caller just set on inject.
    let previousToken = this.auth.token()
    effect(() => {
      const token = this.auth.token()
      if (token === previousToken) {
        return
      }
      previousToken = token
      this.cached$ = null
      this.username.set(null)
      this.isSuperAdmin.set(false)
      this.createdAt.set(null)
      this.organizationId.set(null)
      this.isOrganizationAdmin.set(false)
      this.language.set(null)
      // Signing out, or another account signing in: back to what the browser asks for, so the
      // previous user's language does not linger on the public pages or on a new account.
      this.showBrowserLanguage()
    })
  }

  /** Cached per token: multiple callers (the shell, route guards) share one request. */
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
          this.applyAccountLanguage(me.language)
        }),
        // Never cache a failure — a transient error must not permanently strand the user.
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
   * The account's language wins over the browser's. An account that never chose one (`null`, not
   * `undefined`, which is a server that predates the setting) takes the browser's language, and
   * keeps it: that first sign-in records it on the account, so later ones do not depend on the browser.
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
      // Best effort: the next sign-in tries again, and the interface is already in that language.
      error: () => undefined,
    })
  }

  private showBrowserLanguage(): void {
    const browser = detectBrowserLanguage()
    if (browser !== this.languageService.language()) {
      void this.languageService.use(browser)
    }
  }

  changePassword(currentPassword: string, newPassword: string): Observable<void> {
    return this.port.changePassword(currentPassword, newPassword)
  }

  /** Saves the choice on the account, so it follows the user to their other devices. */
  setLanguage(language: string): Observable<void> {
    return this.port.setLanguage(language).pipe(tap(() => this.language.set(language)))
  }
}
