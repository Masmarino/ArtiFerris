import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  afterNextRender,
  inject,
  signal,
} from '@angular/core'
import { takeUntilDestroyed } from '@angular/core/rxjs-interop'
import { NavigationStart, Router, RouterLink, RouterLinkActive } from '@angular/router'
import { TranslocoPipe } from '@jsverse/transloco'
import { Button, Icon } from '@masmarino/gabarit'
import { filter } from 'rxjs'
import { injectActiveLang } from '../../i18n/active-lang'
import { APP_URL, DOCS_URL, GITHUB_URL } from '../../shared/links'
import { LanguageSwitcher } from '../language-switcher/language-switcher'
import { ThemeToggle } from '../theme-toggle/theme-toggle'

// Scroll distance after which the bar gets its background and rule.
const SCROLLED_AFTER_PX = 8

@Component({
  selector: 'app-header',
  imports: [
    RouterLink,
    RouterLinkActive,
    TranslocoPipe,
    Button,
    Icon,
    LanguageSwitcher,
    ThemeToggle,
  ],
  templateUrl: './header.html',
  styleUrl: './header.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '(document:keydown.escape)': 'closeMenu()' },
})
export class Header {
  protected readonly lang = injectActiveLang()
  protected readonly menuOpen = signal(false)
  /** Drives the `is-scrolled` background of the bar. */
  protected readonly scrolled = signal(false)

  protected readonly appUrl = APP_URL
  protected readonly docsUrl = DOCS_URL
  protected readonly githubUrl = GITHUB_URL

  constructor() {
    const destroyRef = inject(DestroyRef)
    inject(Router)
      .events.pipe(
        filter((event) => event instanceof NavigationStart),
        takeUntilDestroyed(destroyRef),
      )
      .subscribe(() => this.closeMenu())

    afterNextRender(() => {
      let frame = 0
      const update = () => {
        frame = 0
        this.scrolled.set(window.scrollY > SCROLLED_AFTER_PX)
      }
      // At most one scroll read per frame.
      const onScroll = () => {
        if (!frame) frame = requestAnimationFrame(update)
      }
      update()
      window.addEventListener('scroll', onScroll, { passive: true })
      destroyRef.onDestroy(() => {
        window.removeEventListener('scroll', onScroll)
        cancelAnimationFrame(frame)
      })
    })
  }

  protected toggleMenu(): void {
    this.menuOpen.update((open) => !open)
  }

  protected closeMenu(): void {
    this.menuOpen.set(false)
  }
}
