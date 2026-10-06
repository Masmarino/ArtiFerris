import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  PLATFORM_ID,
  inject,
  viewChild,
} from '@angular/core'
import { isPlatformBrowser } from '@angular/common'
import { toSignal } from '@angular/core/rxjs-interop'
import { NavigationEnd, Router, RouterLink } from '@angular/router'
import { TranslocoPipe } from '@jsverse/transloco'
import { Icon } from '@masmarino/gabarit'
import { filter, map } from 'rxjs'
import { injectActiveLang } from '../../i18n/active-lang'
import { LANGS, LANG_NAMES, Lang, PAGES, pageIdFromPath } from '../../i18n/languages'

/**
 * Links to the same page in every language, inside a <details>. Links rather than buttons, so they work before
 * hydration and crawlers can follow them.
 */
@Component({
  selector: 'app-language-switcher',
  imports: [RouterLink, TranslocoPipe, Icon],
  templateUrl: './language-switcher.html',
  styleUrl: './language-switcher.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '(document:click)': 'closeOnOutsideClick($event)',
    '(document:keydown.escape)': 'closeWithEscape()',
  },
})
export class LanguageSwitcher {
  private readonly router = inject(Router)
  private readonly isBrowser = isPlatformBrowser(inject(PLATFORM_ID))
  private readonly details = viewChild<ElementRef<HTMLDetailsElement>>('details')
  private readonly summary = viewChild<ElementRef<HTMLElement>>('summary')

  protected readonly langs = LANGS
  protected readonly names = LANG_NAMES
  protected readonly activeLang = injectActiveLang()

  // Tracked so each link points at the page being shown.
  private readonly pageId = toSignal(
    this.router.events.pipe(
      filter((event) => event instanceof NavigationEnd),
      map((event) => pageIdFromPath(event.urlAfterRedirects)),
    ),
    { initialValue: pageIdFromPath(this.router.url) },
  )

  protected commands(lang: Lang): string[] {
    const path = PAGES[this.pageId()]
    return path ? ['/', lang, path] : ['/', lang]
  }

  protected close(): void {
    const element = this.details()?.nativeElement
    if (element?.open) element.open = false
  }

  /** Escape closes the list and brings the focus back to the button, so a keyboard user does not lose their place. */
  protected closeWithEscape(): void {
    const element = this.details()?.nativeElement
    if (!element?.open) return
    element.open = false
    this.summary()?.nativeElement.focus()
  }

  protected closeOnOutsideClick(event: Event): void {
    if (!this.isBrowser) return
    const element = this.details()?.nativeElement
    if (element?.open && !element.contains(event.target as Node)) element.open = false
  }
}
