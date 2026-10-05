import { Routes } from '@angular/router'
import { docsRoutes } from '@masmarino/gabarit/docs'

// Loaded with loadChildren: the reader's entry brings marked and DOMPurify, which stay out of the
// initial bundle.

/** Without a session: under the public pages' bar. */
export const PUBLIC_DOCS_ROUTES: Routes = docsRoutes(() =>
  import('./public-docs-page').then((m) => m.PublicDocsPage),
)

/** With one: inside the shell. */
export const SHELL_DOCS_ROUTES: Routes = docsRoutes(() =>
  import('./shell-docs-page').then((m) => m.ShellDocsPage),
)
