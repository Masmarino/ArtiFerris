/** A controllable IntersectionObserver for tests: nothing intersects until a test says so. */
export class FakeIntersectionObserver {
  static instances: FakeIntersectionObserver[] = []

  readonly observed = new Set<Element>()

  constructor(private readonly callback: IntersectionObserverCallback) {
    FakeIntersectionObserver.instances.push(this)
  }

  observe(element: Element): void {
    this.observed.add(element)
  }

  unobserve(element: Element): void {
    this.observed.delete(element)
  }

  disconnect(): void {
    this.observed.clear()
  }

  /** Reports that an observed element entered (or left) the viewport. */
  trigger(element: Element, isIntersecting: boolean): void {
    this.callback(
      [{ target: element, isIntersecting } as IntersectionObserverEntry],
      this as unknown as IntersectionObserver,
    )
  }

  static reset(): void {
    FakeIntersectionObserver.instances = []
  }
}

/** Makes `matchMedia` match the reduced-motion, dark-scheme and hover queries on request; any other query does not match. */
export function stubMatchMedia(
  options: { reducedMotion?: boolean; dark?: boolean; hover?: boolean } = {},
): void {
  const answers: [string, boolean | undefined][] = [
    ['prefers-reduced-motion', options.reducedMotion],
    ['prefers-color-scheme: dark', options.dark],
    ['hover: hover', options.hover],
  ]
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    value: (query: string) => ({
      matches: !!answers.find(([name]) => query.includes(name))?.[1],
      media: query,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
    }),
  })
}
