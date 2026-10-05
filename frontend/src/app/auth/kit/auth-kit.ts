import type { Provider } from '@angular/core'
import {
  AUTH_LABELS,
  AUTH_PORT as KIT_AUTH_PORT,
  MFA_PORT as KIT_MFA_PORT,
} from '@masmarino/gabarit/auth'
import { TOTP_QR_RENDERER } from '@masmarino/gabarit/mfa-enrollment'
import { activeLanguage } from '../../shared/i18n/translator'
import { KitAuthAdapter } from './kit-auth.adapter'
import { KitMfaAdapter } from './kit-mfa.adapter'
import { kitLabelsFor } from './labels/kit-labels'
import { renderTotpQr } from './totp-qr-renderer'

/**
 * Gabarit's auth kit wired to our API, a local QR renderer and the strings of the active language,
 * read when a page opens (a sign-in page, or the account's security settings). Listed in each
 * page's own providers, so the nested pieces (enrolment, QR, backup codes) get them too.
 */
export function provideAuthKit(): Provider[] {
  return [
    { provide: KIT_AUTH_PORT, useExisting: KitAuthAdapter },
    { provide: KIT_MFA_PORT, useExisting: KitMfaAdapter },
    { provide: TOTP_QR_RENDERER, useValue: renderTotpQr },
    { provide: AUTH_LABELS, useFactory: () => kitLabelsFor(activeLanguage()) },
  ]
}
