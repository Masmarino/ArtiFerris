import { ChangeDetectionStrategy, Component, inject } from '@angular/core'
import { Router, RouterLink } from '@angular/router'
import { AuthFooterLink } from '@masmarino/gabarit/auth'
import { AuthActivate } from '@masmarino/gabarit/auth-activate'
import { Button } from '@masmarino/gabarit/button'
import { TranslocoPipe } from '@jsverse/transloco'
import { provideAuthKit } from '../kit/auth-kit'
import { consumeLinkToken } from '../kit/link-token'
import { GitField } from '@masmarino/gabarit/git-field'

/**
 * Owns the `/activate#token=…` URL of an invitation mail around Gabarit's activation. The
 * administrator invites by e-mail only, so the invitee chooses their username here, with their
 * password. The token is read once, then dropped from the address bar; a missing or malformed one
 * shows the kit's dead-link view without any request.
 */
@Component({
  selector: 'app-activate-page',
  standalone: true,
  imports: [GitField, AuthActivate, AuthFooterLink, Button, RouterLink, TranslocoPipe],
  providers: [provideAuthKit()],
  host: { class: 'auth-layout' },
  template: `
    <gbt-auth-activate [token]="token" [chooseUsername]="true" (signIn)="toSignIn()">
      <gbt-git-field auth-backdrop />
      <img auth-logo src="/api/branding/logo" [alt]="'auth.login.logoAlt' | transloco" />
      <a gbtButton variant="link" gbtAuthFooterLink routerLink="/login">{{
        'auth.login.submit' | transloco
      }}</a>
    </gbt-auth-activate>
  `,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ActivatePage {
  private readonly router = inject(Router)

  protected readonly token = consumeLinkToken('/activate')

  protected toSignIn(): void {
    void this.router.navigateByUrl('/login')
  }
}
