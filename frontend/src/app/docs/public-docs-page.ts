import { ChangeDetectionStrategy, Component } from '@angular/core'
import { DocsPage } from '@masmarino/gabarit/docs'
import { PublicLayout } from '../public/public-layout/public-layout'
import { provideArtiferrisDocs } from './docs-providers'

/** The documentation for a visitor without a session: under the public pages' bar, the whole width. */
@Component({
  selector: 'app-public-docs-page',
  standalone: true,
  imports: [PublicLayout, DocsPage],
  providers: [provideArtiferrisDocs()],
  template: '<app-public-layout fullWidth><gbt-docs-page /></app-public-layout>',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PublicDocsPage {}
