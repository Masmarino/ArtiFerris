import { type Provider, inject } from '@angular/core'
import { DOCS_LABELS, DOCS_TITLE, provideDocs } from '@masmarino/gabarit/docs'
import { activeLanguage } from '../shared/i18n/translator'
import { PageTitleService } from '../shell/page-title.service'
import { docsLabelsFor } from './docs-labels'

/**
 * Gabarit's documentation reader over `public/docs/`: the reader in the active language, its page titles in the
 * header and the browser tab, and the French callouts of our pages (`> **Note** …`, `> **Attention** …`).
 */
export function provideArtiferrisDocs(): Provider[] {
  return [
    provideDocs({ callouts: { note: 'note', attention: 'warning' } }),
    { provide: DOCS_LABELS, useFactory: () => docsLabelsFor(activeLanguage()) },
    {
      provide: DOCS_TITLE,
      useFactory: () => {
        const pageTitle = inject(PageTitleService)
        return (title: string) => pageTitle.title.set(title)
      },
    },
  ]
}
