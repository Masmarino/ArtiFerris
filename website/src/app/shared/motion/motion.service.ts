import { Injectable, PLATFORM_ID, inject, signal } from '@angular/core'
import { DOCUMENT, isPlatformBrowser } from '@angular/common'

const REDUCED_MOTION = '(prefers-reduced-motion: reduce)'

/**
 * Reduced-motion preference and tab visibility, for code that decides what to render or observe. The stylesheet
 * handles both on its own (media query, data-tab-hidden).
 */
@Injectable({ providedIn: 'root' })
export class MotionService {
  private readonly document = inject(DOCUMENT)
  private readonly isBrowser = isPlatformBrowser(inject(PLATFORM_ID))

  /** True when the visitor asked the system for less motion. False on the server. */
  readonly reducedMotion = signal(false)

  init(): void {
    if (!this.isBrowser) return
    const root = this.document.documentElement

    // js-ready tells theme-init.js that the app started (see there).
    root.classList.add('js', 'js-ready')

    const media = window.matchMedia(REDUCED_MOTION)
    this.reducedMotion.set(media.matches)
    media.addEventListener('change', (event) => this.reducedMotion.set(event.matches))

    // Loops pause in background tabs through data-tab-hidden, not through browser throttling.
    const sync = () => root.toggleAttribute('data-tab-hidden', this.document.hidden)
    sync()
    this.document.addEventListener('visibilitychange', sync)
  }
}
