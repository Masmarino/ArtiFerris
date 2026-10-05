import type { AuthLabels } from '@masmarino/gabarit/auth'
import { Language } from '../../../shared/i18n/languages'
import { DE_AUTH_LABELS } from './auth-labels.de'
import { EN_AUTH_LABELS } from './auth-labels.en'
import { ES_AUTH_LABELS } from './auth-labels.es'
import { FR_AUTH_LABELS } from './auth-labels.fr'
import { IT_AUTH_LABELS } from './auth-labels.it'
import { DE_SETTINGS_LABELS } from './settings-labels.de'
import { ES_SETTINGS_LABELS } from './settings-labels.es'
import { FR_SETTINGS_LABELS } from './settings-labels.fr'
import { IT_SETTINGS_LABELS } from './settings-labels.it'

/** The parts of the kit our sign-in pages show: the pages, and what they nest (enrolment, QR, codes). */
export type KitPart =
  'totpQr' | 'backupCodes' | 'mfaEnrollment' | 'login' | 'register' | 'activate' | 'resetPassword'

/** Every string of those parts, so a translation cannot miss one. */
export type KitLabels = { [K in KitPart]-?: Required<NonNullable<AuthLabels[K]>> }

/** The account's security settings: the authenticator app and its backup codes, the passkeys. */
export type SettingsLabels = {
  [K in 'mfaSettings' | 'passkeySettings']-?: Required<NonNullable<AuthLabels[K]>>
}

/** English is the kit's own wording; ours only names the product and its files. */
const LABELS: Record<Language, AuthLabels> = {
  en: EN_AUTH_LABELS,
  fr: { ...FR_AUTH_LABELS, ...FR_SETTINGS_LABELS },
  es: { ...ES_AUTH_LABELS, ...ES_SETTINGS_LABELS },
  de: { ...DE_AUTH_LABELS, ...DE_SETTINGS_LABELS },
  it: { ...IT_AUTH_LABELS, ...IT_SETTINGS_LABELS },
}

export function kitLabelsFor(language: Language): AuthLabels {
  return LABELS[language]
}
