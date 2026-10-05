import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  inject,
  input,
} from '@angular/core'
import { toSignal } from '@angular/core/rxjs-interop'
import { NavigationEnd, Router, RouterLink, RouterLinkActive } from '@angular/router'
import { filter, map } from 'rxjs'
import { Button } from '@masmarino/gabarit/button'
import { AuthService } from '../../auth/application/auth.service'
import { catalogNameFromUrl } from '../catalog/domain/catalog.registry'
import { SuggestSearchBox } from '../catalog/suggest-search-box/suggest-search-box'

const REPOSITORY_URL = 'https://github.com/Masmarino/ArtiFerris'

/** The public pages' frame: the graphite bar (logo, quick search, GitHub, documentation, sign-in) above the page. */
@Component({
  selector: 'app-public-layout',
  standalone: true,
  imports: [TranslocoPipe, RouterLink, RouterLinkActive, Button, SuggestSearchBox],
  templateUrl: './public-layout.html',
  styleUrl: './public-layout.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PublicLayout {
  private readonly router = inject(Router)
  private readonly auth = inject(AuthService)
  private readonly url = toSignal(
    this.router.events.pipe(
      filter((event) => event instanceof NavigationEnd),
      map(() => this.router.url),
    ),
    { initialValue: this.router.url },
  )
  // Signed-in visitors can land here too: give them a way back to their dashboard.
  readonly isAuthenticated = this.auth.isAuthenticated
  /** Pages that fill the width, like the documentation; the others get a centred column. */
  readonly fullWidth = input(false, { transform: booleanAttribute })

  protected readonly repositoryUrl = REPOSITORY_URL
  /** Back to this page after signing in, unless it is the catalog's home. */
  protected readonly loginParams = computed(() => {
    const url = this.url()
    return url === '/' || url === '/explorer' ? {} : { returnUrl: url }
  })

  search(text: string): void {
    const target = catalogNameFromUrl(this.router.url) ?? 'explorer'
    void this.router.navigate(['/', target], { queryParams: { q: text.trim() || null } })
  }
}
