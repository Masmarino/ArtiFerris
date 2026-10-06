import { Injectable, PLATFORM_ID, inject } from '@angular/core'
import { isPlatformBrowser } from '@angular/common'

type Listener = (visible: boolean) => void

interface Watch {
  listener: Listener
  once: boolean
}

/**
 * One IntersectionObserver for the whole site. Elements ask to be told when they enter the viewport, either once
 * (reveals are released after the first time) or continuously (loops that pause off screen).
 */
@Injectable({ providedIn: 'root' })
export class ViewportObserver {
  private readonly isBrowser = isPlatformBrowser(inject(PLATFORM_ID))
  // An element can have several watches: a reveal and a loop tracker can sit on the same host.
  private readonly watches = new Map<Element, Watch[]>()
  private observer: IntersectionObserver | null = null

  /** False on the server and in browsers without IntersectionObserver: callers then show the final state. */
  get supported(): boolean {
    return this.isBrowser && typeof IntersectionObserver !== 'undefined'
  }

  observeOnce(element: Element, listener: Listener): void {
    this.add(element, { listener, once: true })
  }

  observe(element: Element, listener: Listener): void {
    this.add(element, { listener, once: false })
  }

  /** Stops every watch on the element. */
  unobserve(element: Element): void {
    if (this.watches.delete(element)) this.observer?.unobserve(element)
  }

  private add(element: Element, watch: Watch): void {
    if (!this.supported) return
    // The bottom margin makes things appear a bit after they enter, not while they're a single pixel high.
    this.observer ??= new IntersectionObserver((entries) => this.handle(entries), {
      rootMargin: '0px 0px -8% 0px',
    })
    const existing = this.watches.get(element)
    if (existing) existing.push(watch)
    else this.watches.set(element, [watch])
    this.observer.observe(element)
  }

  private handle(entries: IntersectionObserverEntry[]): void {
    for (const entry of entries) {
      const watches = this.watches.get(entry.target)
      if (!watches) continue
      for (const watch of [...watches]) {
        if (watch.once) {
          if (!entry.isIntersecting) continue
          watches.splice(watches.indexOf(watch), 1)
        }
        watch.listener(entry.isIntersecting)
      }
      // Nothing left to tell this element: release it.
      if (watches.length === 0) this.unobserve(entry.target)
    }
  }
}
