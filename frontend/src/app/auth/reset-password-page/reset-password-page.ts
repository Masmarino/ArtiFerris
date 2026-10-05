import { ChangeDetectionStrategy, Component, inject } from '@angular/core'
import { Router, RouterLink } from '@angular/router'
import { AuthFooterLink } from '@masmarino/gabarit/auth'
import { AuthResetPassword } from '@masmarino/gabarit/auth-reset-password'
import { Button } from '@masmarino/gabarit/button'
import { GitField } from '@masmarino/gabarit/git-field'
import { TranslocoPipe } from '@jsverse/transloco'
import { provideAuthKit } from '../kit/auth-kit'
import { consumeLinkToken } from '../kit/link-token'

/**
 * Owns the `/reset-password#token=…` URL of the mail an administrator's reset sends, as FerrisGit
 * does, around Gabarit's page. The token is read once, then dropped from the address bar; a missing
 * or malformed one shows the kit's dead-link view without any request.
 */
@Component({
  selector: 'app-reset-password-page',
  standalone: true,
  imports: [GitField, AuthResetPassword, AuthFooterLink, Button, RouterLink, TranslocoPipe],
  providers: [provideAuthKit()],
  host: { class: 'auth-layout' },
  template: `
    <gbt-auth-reset-password [token]="token" (signIn)="toSignIn()">
      <gbt-git-field auth-backdrop />
      <img auth-logo src="/api/branding/logo" [alt]="'auth.login.logoAlt' | transloco" />
      <a gbtButton variant="link" gbtAuthFooterLink routerLink="/login">{{
        'auth.login.submit' | transloco
      }}</a>
    </gbt-auth-reset-password>
  `,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ResetPasswordPage {
  private readonly router = inject(Router)

  protected readonly token = consumeLinkToken('/reset-password')

  protected toSignIn(): void {
    void this.router.navigateByUrl('/login')
  }
}
