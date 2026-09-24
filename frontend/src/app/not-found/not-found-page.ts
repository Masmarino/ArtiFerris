import { ChangeDetectionStrategy, Component, inject } from '@angular/core'
import { RouterLink } from '@angular/router'
import { Card } from '@masmarino/gabarit'
import { PublicLayout } from '../public/public-layout/public-layout'
import { PageTitleService } from '../shell/page-title.service'

@Component({
  selector: 'app-not-found-page',
  standalone: true,
  imports: [Card, PublicLayout, RouterLink],
  templateUrl: './not-found-page.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class NotFoundPage {
  constructor() {
    inject(PageTitleService).title.set('Page introuvable')
  }
}
