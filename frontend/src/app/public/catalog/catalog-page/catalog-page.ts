import { ChangeDetectionStrategy, Component, inject } from '@angular/core'
import { ActivatedRoute } from '@angular/router'
import { PageTitleService } from '../../../shell/page-title.service'
import { PublicLayout } from '../../public-layout/public-layout'
import { CatalogSearch } from '../catalog-search/catalog-search'
import { CatalogFormat } from '../domain/catalog.entity'

const BLURBS: Record<CatalogFormat, string> = {
  npm: "Tous les paquets npm publics hébergés sur cette instance. On installe toujours depuis l'URL du propriétaire.",
  docker:
    "Toutes les images Docker publiques hébergées sur cette instance. On tire toujours depuis l'URL du propriétaire.",
}

/** Route data supplies `catalogName` and `format`, see `catalogRoutes` in app.routes.ts. */
@Component({
  selector: 'app-catalog-page',
  standalone: true,
  imports: [CatalogSearch, PublicLayout],
  templateUrl: './catalog-page.html',
  styleUrl: './catalog-page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class CatalogPage {
  private readonly data = inject(ActivatedRoute).snapshot.data

  readonly name = this.data['catalogName'] as string
  readonly format = this.data['format'] as CatalogFormat
  readonly blurb = BLURBS[this.format]

  constructor() {
    inject(PageTitleService).title.set(this.name)
  }
}
