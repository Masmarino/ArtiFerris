declare const jsdom: { window: Window } | undefined

if (typeof jsdom !== 'undefined') {
  Object.defineProperty(globalThis, 'localStorage', {
    value: jsdom.window.localStorage,
    configurable: true,
  })
  Object.defineProperty(globalThis, 'sessionStorage', {
    value: jsdom.window.sessionStorage,
    configurable: true,
  })
}
