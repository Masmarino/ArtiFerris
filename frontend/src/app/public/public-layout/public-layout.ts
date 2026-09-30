import { TranslocoPipe } from '@jsverse/transloco'
import { ChangeDetectionStrategy, Component, inject } from '@angular/core'
import { Router, RouterLink } from '@angular/router'
import { AuthService } from '../../auth/application/auth.service'
import { catalogNameFromUrl } from '../catalog/domain/catalog.registry'
import { SuggestSearchBox } from '../catalog/suggest-search-box/suggest-search-box'

@Component({
  selector: 'app-public-layout',
  standalone: true,
  imports: [TranslocoPipe, RouterLink, SuggestSearchBox],
  templateUrl: './public-layout.html',
  styleUrl: './public-layout.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PublicLayout {
  private readonly router = inject(Router)
  private readonly auth = inject(AuthService)
  // The explorer is now the site's home page, so a signed-in visitor can land here too — give
  // them a way back to their own dashboard instead of an inapplicable "Se connecter" link.
  readonly isAuthenticated = this.auth.isAuthenticated

  search(text: string): void {
    const target = catalogNameFromUrl(this.router.url) ?? 'explorer'
    void this.router.navigate(['/', target], { queryParams: { q: text.trim() || null } })
  }
}
