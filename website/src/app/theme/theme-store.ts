import { Injectable, PLATFORM_ID, computed, inject, signal } from '@angular/core'
import { isPlatformBrowser } from '@angular/common'

export type ThemePreference = 'light' | 'dark' | 'system'
export type EffectiveTheme = 'light' | 'dark'

// public/theme-init.js reads the same key.
export const THEME_STORAGE_KEY = 'artiferris-site-theme'

// Longer than the colour fade in styles.scss (--dur-2) so that the fade can finish.
const THEME_TRANSITION_MS = 400

function readStoredPreference(): ThemePreference {
  try {
    const stored = window.localStorage.getItem(THEME_STORAGE_KEY)
    return stored === 'light' || stored === 'dark' ? stored : 'system'
  } catch {
    // Storage can throw in private windows or when site data is blocked.
    return 'system'
  }
}

@Injectable({ providedIn: 'root' })
export class ThemeStore {
  private readonly isBrowser = isPlatformBrowser(inject(PLATFORM_ID))
  private readonly systemPrefersDark = signal(false)
  private transitionTimer = 0

  readonly preference = signal<ThemePreference>('system')
  readonly effective = computed<EffectiveTheme>(() => {
    const preference = this.preference()
    if (preference === 'system') return this.systemPrefersDark() ? 'dark' : 'light'
    return preference
  })

  init(): void {
    if (!this.isBrowser) return

    const media = window.matchMedia('(prefers-color-scheme: dark)')
    this.systemPrefersDark.set(media.matches)
    media.addEventListener('change', (event) => this.systemPrefersDark.set(event.matches))

    this.preference.set(readStoredPreference())
  }

  toggle(): void {
    this.setPreference(this.effective() === 'dark' ? 'light' : 'dark')
  }

  setPreference(preference: ThemePreference): void {
    this.preference.set(preference)
    if (!this.isBrowser) return
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, preference)
    } catch {
      // The choice still applies to this page view.
    }
    const root = document.documentElement
    // theme-transition fades the colours (styles.scss). Only a click on the toggle gets here, so the first paint
    // is never animated.
    root.classList.add('theme-transition')
    if (preference === 'system') root.removeAttribute('data-theme')
    else root.setAttribute('data-theme', preference)
    window.clearTimeout(this.transitionTimer)
    this.transitionTimer = window.setTimeout(
      () => root.classList.remove('theme-transition'),
      THEME_TRANSITION_MS,
    )
  }
}
