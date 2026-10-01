import { Injectable, effect, inject, signal } from '@angular/core'
import { Title } from '@angular/platform-browser'

// Current page title for the header: static routes set it from data.title, data-dependent pages
// override it.
@Injectable({ providedIn: 'root' })
export class PageTitleService {
  private readonly browserTitle = inject(Title)

  readonly title = signal('')

  constructor() {
    // Otherwise the tab title never changes between routes.
    effect(() => {
      const title = this.title()
      this.browserTitle.setTitle(
        title ? `${title} · ArtiFerris` : 'ArtiFerris · Artifact Repository',
      )
    })
  }
}
