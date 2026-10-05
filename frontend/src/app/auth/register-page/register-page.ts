import { ChangeDetectionStrategy, Component, inject } from '@angular/core'
import { Router, RouterLink } from '@angular/router'
import { AuthFooterLink } from '@masmarino/gabarit/auth'
import { AuthRegister } from '@masmarino/gabarit/auth-register'
import { Button } from '@masmarino/gabarit/button'
import { TranslocoPipe } from '@jsverse/transloco'
import { provideAuthKit } from '../kit/auth-kit'
import { GitField } from '@masmarino/gabarit/git-field'

/**
 * Owns the `/register` URL around Gabarit's registration: the new account is signed in and taken
 * through the mandatory second factor, then lands on the home page.
 */
@Component({
  selector: 'app-register-page',
  standalone: true,
  imports: [GitField, AuthRegister, AuthFooterLink, Button, RouterLink, TranslocoPipe],
  providers: [provideAuthKit()],
  host: { class: 'auth-layout' },
  template: `
    <gbt-auth-register (registered)="toHome()" (signIn)="toSignIn()">
      <gbt-git-field auth-backdrop />
      <img auth-logo src="/api/branding/logo" [alt]="'auth.login.logoAlt' | transloco" />
      <a gbtButton variant="link" gbtAuthFooterLink routerLink="/login">{{
        'auth.login.submit' | transloco
      }}</a>
    </gbt-auth-register>
  `,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class RegisterPage {
  private readonly router = inject(Router)

  protected toHome(): void {
    void this.router.navigateByUrl('/')
  }

  protected toSignIn(): void {
    void this.router.navigateByUrl('/login')
  }
}
