import { OverloadMessages } from '../../../shared/api-error'

/** How every public catalog view words a 429 or 503 (translation keys). */
export const CATALOG_OVERLOAD: OverloadMessages = {
  tooManyRequests: 'catalog.overload.tooManyRequests',
  busy: 'catalog.overload.busy',
}
