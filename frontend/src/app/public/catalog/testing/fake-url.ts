import { EnvironmentProviders, Provider, inject, provideAppInitializer } from '@angular/core'
import {
  ActivatedRoute,
  Data,
  NavigationExtras,
  ParamMap,
  Router,
  convertToParamMap,
} from '@angular/router'
import { BehaviorSubject, Observable, of } from 'rxjs'

/** Stands in for the URL: the query string is read here and written back through `navigateFake`, the path params stay fixed. */
export class FakeActivatedRoute {
  private readonly params$: BehaviorSubject<ParamMap>

  readonly queryParamMap: Observable<ParamMap>
  readonly paramMap: Observable<ParamMap>
  readonly snapshot: { data: Data }

  constructor(
    initial: Record<string, string> = {},
    data: Data = {},
    pathParams: Record<string, string> = {},
  ) {
    this.params$ = new BehaviorSubject(convertToParamMap(initial))
    this.queryParamMap = this.params$.asObservable()
    this.paramMap = of(convertToParamMap(pathParams))
    this.snapshot = { data }
  }

  get current(): Record<string, string> {
    const map = this.params$.value
    return Object.fromEntries(map.keys.map((key) => [key, map.get(key)!]))
  }

  set(params: Record<string, string>): void {
    this.params$.next(convertToParamMap(params))
  }
}

export function navigateFake(
  route: FakeActivatedRoute,
  extras?: NavigationExtras,
): Promise<boolean> {
  const next = { ...route.current }
  for (const [key, value] of Object.entries(extras?.queryParams ?? {})) {
    if (value === null || value === undefined) {
      delete next[key]
    } else {
      next[key] = String(value)
    }
  }
  route.set(next)
  return Promise.resolve(true)
}

export function fakeUrlProviders(
  initial: Record<string, string> = {},
  routeData: Data = {},
  pathParams: Record<string, string> = {},
): (Provider | EnvironmentProviders)[] {
  return [
    {
      provide: ActivatedRoute,
      useFactory: () => new FakeActivatedRoute(initial, routeData, pathParams),
    },
    provideAppInitializer(() => {
      const router = inject(Router)
      const route = inject(ActivatedRoute) as unknown as FakeActivatedRoute
      router.navigate = (_commands, extras) => navigateFake(route, extras)
    }),
  ]
}
