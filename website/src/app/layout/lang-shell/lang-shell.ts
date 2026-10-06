import { ChangeDetectionStrategy, Component, DestroyRef, PLATFORM_ID, inject } from '@angular/core'
import { DOCUMENT, isPlatformBrowser } from '@angular/common'
import { takeUntilDestroyed } from '@angular/core/rxjs-interop'
import { NavigationEnd, Router, RouterOutlet } from '@angular/router'
import { TranslocoPipe } from '@jsverse/transloco'
import { filter, skip } from 'rxjs'
import { Footer } from '../footer/footer'
import { Header } from '../header/header'

const MAIN_ID = 'main-content'

/** Header, footer and skip link around the pages of one language. */
@Component({
  selector: 'app-lang-shell',
  imports: [RouterOutlet, TranslocoPipe, Header, Footer],
  templateUrl: './lang-shell.html',
  styleUrl: './lang-shell.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class LangShell {
  private readonly document = inject(DOCUMENT)

  constructor() {
    if (!isPlatformBrowser(inject(PLATFORM_ID))) return
    // Move the focus after a client-side navigation, or it stays on a link of the page that was just replaced. Skip the
    // first NavigationEnd: on the initial load the browser has already placed it.
    inject(Router)
      .events.pipe(
        filter((event) => event instanceof NavigationEnd),
        skip(1),
        takeUntilDestroyed(inject(DestroyRef)),
      )
      .subscribe(() => this.focusMain())
  }

  // With <base href="/"> a plain #main-content link would go to the site root.
  protected skipToContent(event: Event): void {
    event.preventDefault()
    this.focusMain()
  }

  private focusMain(): void {
    this.document.getElementById(MAIN_ID)?.focus()
  }
}
