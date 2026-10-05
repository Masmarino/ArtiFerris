import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core'
import { NgTemplateOutlet } from '@angular/common'
import { ActivatedRoute, Router, RouterLink } from '@angular/router'
import { Alert } from '@masmarino/gabarit/alert'
import {
  AUTH_LABELS,
  AuthFooterLink,
  AuthPanel,
  DEFAULT_LOGIN_LABELS,
} from '@masmarino/gabarit/auth'
import { AuthLogin } from '@masmarino/gabarit/auth-login'
import { Button } from '@masmarino/gabarit/button'
import { TranslocoPipe } from '@jsverse/transloco'
import { AuthService } from '../application/auth.service'
import { safeReturnUrl } from '../domain/return-url'
import {
  SESSIONS_ENDED_MESSAGE_KEY,
  SESSIONS_ENDED_QUERY_PARAM,
  SESSIONS_ENDED_REASON,
} from '../domain/sessions-ended'
import { provideAuthKit } from '../kit/auth-kit'
import { KitAuthAdapter } from '../kit/kit-auth.adapter'
import { GitField } from '@masmarino/gabarit/git-field'
import { t } from '../../shared/i18n/translator'

/**
 * Owns the `/login` URL around Gabarit's sign-in: the notices (sessions ended, a sign-in link that
 * failed), the organisation's single sign-on (an OIDC organisation signs in at its identity
 * provider; an LDAP one through the same form, see KitAuthAdapter), the OIDC return (`#token=`),
 * and where to go once signed in (`?returnUrl=`).
 */
@Component({
  selector: 'app-login-page',
  standalone: true,
  imports: [
    GitField,
    AuthLogin,
    AuthPanel,
    AuthFooterLink,
    Alert,
    Button,
    RouterLink,
    NgTemplateOutlet,
    TranslocoPipe,
  ],
  providers: [provideAuthKit()],
  host: { class: 'auth-layout' },
  templateUrl: './login-page.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class LoginPage implements OnInit {
  private readonly auth = inject(AuthService)
  private readonly kit = inject(KitAuthAdapter)
  private readonly router = inject(Router)
  private readonly queryParams = inject(ActivatedRoute).snapshot.queryParamMap
  private readonly returnUrl = safeReturnUrl(this.queryParams.get('returnUrl'))

  protected readonly heading = inject(AUTH_LABELS).login?.heading ?? DEFAULT_LOGIN_LABELS.heading
  protected readonly sessionsEndedMessage =
    this.queryParams.get(SESSIONS_ENDED_QUERY_PARAM) === SESSIONS_ENDED_REASON
      ? t(SESSIONS_ENDED_MESSAGE_KEY)
      : null
  protected readonly ssoLinkInvalid = signal(false)
  protected readonly oidc = signal(false)
  /** Back from the identity provider with a session: nothing to show on the way to the app. */
  protected readonly redirecting = signal(false)

  ngOnInit(): void {
    const hashParams = new URLSearchParams(window.location.hash.replace(/^#/, ''))
    const token = hashParams.get('token')
    if (token !== null) {
      // Clear the fragment so the token does not stay in history.
      history.replaceState(null, '', window.location.pathname + window.location.search)
      const completed = token ? this.auth.completeExternalLogin(token) : null
      if (completed) {
        this.redirecting.set(true)
        this.signedIn(completed.returnUrl)
        return
      }
      this.ssoLinkInvalid.set(true)
    } else {
      this.auth.abandonSsoLogin()
    }
    // The kit's form asks for the same settings at the same time: one request for both.
    this.kit.ssoConfig$.subscribe({
      next: (config) => this.oidc.set(config.type === 'oidc'),
      // Local sign-in stays available if this check fails.
      error: () => this.oidc.set(false),
    })
  }

  protected startSso(): void {
    this.auth.beginSsoLogin(this.returnUrl)
  }

  protected signedIn(returnUrl: string | null = this.returnUrl): void {
    void this.router.navigateByUrl(returnUrl ?? '/')
  }
}
