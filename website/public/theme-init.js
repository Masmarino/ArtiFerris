// Loaded from <head> as an external file because the CSP forbids inline scripts. It has to run before the first paint.

// Apply a stored theme before any CSS is used, so a visitor who picked the other theme never sees a flash.
// Without one, the stylesheet follows prefers-color-scheme. Same storage key as src/app/theme/theme-store.ts.
try {
  var storedTheme = localStorage.getItem('artiferris-site-theme')
  if (storedTheme === 'light' || storedTheme === 'dark') {
    document.documentElement.setAttribute('data-theme', storedTheme)
  }
} catch (e) {}

// The stylesheet only hides elements that animate in (the "before it appears" state) under `html.js`, so without
// JavaScript every page shows in full. If the app hasn't set `js-ready` after 6 seconds (bundle blocked or failed),
// `js` is removed and the page falls back to that. MotionService puts both classes back if the app starts late.
;(function () {
  var root = document.documentElement
  root.classList.add('js')
  setTimeout(function () {
    if (!root.classList.contains('js-ready')) root.classList.remove('js')
  }, 6000)
})()
