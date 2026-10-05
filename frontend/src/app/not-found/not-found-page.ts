import { t } from '../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import { ChangeDetectionStrategy, Component, inject } from '@angular/core'
import { Router, RouterLink } from '@angular/router'
import { Button } from '@masmarino/gabarit/button'
import { Card } from '@masmarino/gabarit/card'
import { EmptyState } from '@masmarino/gabarit/empty-state'
import { AuthService } from '../auth/application/auth.service'
import { PublicLayout } from '../public/public-layout/public-layout'
import { PageTitleService } from '../shell/page-title.service'

/**
 * An unknown address, laid out like FerrisGit's: one answer for a missing page and a private one, so
 * a visitor can't tell which. A visitor is offered to sign in and come back here.
 */
@Component({
  selector: 'app-not-found-page',
  standalone: true,
  imports: [TranslocoPipe, Button, Card, EmptyState, PublicLayout, RouterLink],
  templateUrl: './not-found-page.html',
  styleUrl: './not-found-page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class NotFoundPage {
  protected readonly signedIn = inject(AuthService).isAuthenticated
  protected readonly returnUrl = { returnUrl: inject(Router).url }

  constructor() {
    inject(PageTitleService).title.set(t('notFound.heading'))
  }
}
