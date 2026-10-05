import { ChangeDetectionStrategy, Component } from '@angular/core'
import { DocsPage } from '@masmarino/gabarit/docs'
import { provideArtiferrisDocs } from './docs-providers'

/** The documentation for a signed-in user: inside the app's shell. */
@Component({
  selector: 'app-shell-docs-page',
  standalone: true,
  imports: [DocsPage],
  providers: [provideArtiferrisDocs()],
  template: '<gbt-docs-page />',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ShellDocsPage {}
