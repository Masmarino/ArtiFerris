import { OverloadMessages } from '../../../shared/api-error'

/** How every public catalog view words a 429 or 503. */
export const CATALOG_OVERLOAD: OverloadMessages = {
  tooManyRequests: 'Trop de requêtes, patientez un instant puis réessayez.',
  busy: 'Le catalogue est momentanément occupé',
}
