import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core'
import { TranslocoPipe } from '@jsverse/transloco'
import { ThemeStore } from '../../../theme/theme-store'

export type ScreenName = 'repositories' | 'package' | 'scan' | 'organizations' | 'explore' | 'docs'

// A tight crop of the screen, taken at twice the pixel density so the interface text reads at the size it has on the
// page: <name>-detail-light.webp and -dark.webp. The whole 1600x1000 screen is <name>-light.webp and -dark.webp and
// opens from a link. Sizes of the crops: scripts/capture-screens.mjs.
const DETAIL_SIZES: Record<ScreenName, { width: number; height: number }> = {
  repositories: { width: 1860, height: 1240 },
  package: { width: 1860, height: 1330 },
  scan: { width: 1860, height: 1080 },
  organizations: { width: 1920, height: 1120 },
  explore: { width: 1880, height: 1240 },
  docs: { width: 1920, height: 1280 },
}

@Component({
  selector: 'app-screenshot',
  imports: [TranslocoPipe],
  templateUrl: './screenshot.html',
  styleUrl: './screenshot.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class Screenshot {
  private readonly theme = inject(ThemeStore)

  readonly name = input.required<ScreenName>()
  /** The first screenshot of a page loads eagerly with high priority; the others lazily. */
  readonly eager = input(false)

  protected readonly size = computed(() => DETAIL_SIZES[this.name()])
  protected readonly lightSrc = computed(() => `images/screens/${this.name()}-detail-light.webp`)
  protected readonly darkSrc = computed(() => `images/screens/${this.name()}-detail-dark.webp`)
  protected readonly altKey = computed(() => `screens.${this.name()}`)
  protected readonly fullHref = computed(
    () => `images/screens/${this.name()}-${this.theme.effective()}.webp`,
  )

  // The browser picks the dark file from prefers-color-scheme. A theme chosen with the toggle overrides that: the media
  // query becomes "all" (always matches) or "not all" (never).
  protected readonly darkMedia = computed(() => {
    switch (this.theme.preference()) {
      case 'dark':
        return 'all'
      case 'light':
        return 'not all'
      default:
        return '(prefers-color-scheme: dark)'
    }
  })
}
