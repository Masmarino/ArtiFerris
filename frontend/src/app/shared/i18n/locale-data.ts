import { registerLocaleData } from '@angular/common'
import localeDe from '@angular/common/locales/de'
import localeEs from '@angular/common/locales/es'
import localeFr from '@angular/common/locales/fr'
import localeIt from '@angular/common/locales/it'

// English needs no registration: it is the locale data Angular ships built in.
for (const data of [localeFr, localeEs, localeIt, localeDe]) {
  registerLocaleData(data)
}
