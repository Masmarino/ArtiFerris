import type { TotpQrRenderOptions, TotpQrRenderer } from '@masmarino/gabarit/mfa-enrollment'

type QrCodeModule = typeof import('qrcode')

/** `toDataURL` from `qrcode`, whichever way the bundler exposes the CommonJS package. */
export function qrToDataUrl(module: QrCodeModule): QrCodeModule['toDataURL'] {
  return module.toDataURL ?? (module as unknown as { default: QrCodeModule }).default.toDataURL
}

/**
 * Loaded lazily, only the enrolment needs it. `text` holds the user's TOTP secret, so the QR is drawn
 * locally into a `data:` URL: no hosted QR service, no network call.
 */
export const renderTotpQr: TotpQrRenderer = (text: string, options: TotpQrRenderOptions) =>
  import('qrcode').then((module) => qrToDataUrl(module)(text, options))
