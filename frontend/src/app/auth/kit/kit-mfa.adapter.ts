import { HttpClient } from '@angular/common/http'
import { Injectable, inject } from '@angular/core'
import type {
  BackupCodesResult,
  MfaPort as KitMfaPort,
  MfaStatus,
  Passkey,
  PasskeyChallenge,
  TotpEnrollment,
} from '@masmarino/gabarit/auth'
import { Observable, defer, forkJoin, map, share } from 'rxjs'
import { apiPath } from '../../shared/api-path'
import { kitErrors } from './kit-errors'

interface PasskeyRow {
  id: string
  name: string
  created_at: string
  last_used_at: string | null
}

const toPasskey = (row: PasskeyRow): Passkey => ({
  id: row.id,
  name: row.name,
  createdAt: row.created_at,
  lastUsedAt: row.last_used_at,
})

const toCodes = (body: { backup_codes: string[] }): BackupCodesResult => ({
  backupCodes: body.backup_codes,
})

/**
 * Gabarit's security settings (the authenticator app, its backup codes, the passkeys) on our
 * `/api/me/mfa` routes. Every change that needs the password ends the account's sessions on the
 * server; the settings then tell the page, which signs out.
 */
@Injectable({ providedIn: 'root' })
export class KitMfaAdapter implements KitMfaPort {
  private readonly http = inject(HttpClient)

  /** The app and the passkey settings load it together: one round trip for both, a later call asks again. */
  private readonly status$: Observable<MfaStatus> = defer(() =>
    forkJoin([
      this.http.get<{ totp_enabled: boolean; backup_codes_remaining: number }>('/api/me/mfa'),
      this.http.get<PasskeyRow[]>('/api/me/mfa/passkey'),
    ]),
  ).pipe(
    kitErrors,
    map(([status, passkeys]) => ({
      totpEnabled: status.totp_enabled,
      backupCodesRemaining: status.backup_codes_remaining,
      passkeys: passkeys.map(toPasskey),
    })),
    share(),
  )

  status(): Observable<MfaStatus> {
    return this.status$
  }

  enroll(currentPassword: string): Observable<TotpEnrollment> {
    return this.http
      .post<{ secret: string; otpauth_url: string }>('/api/me/mfa/totp/enroll', {
        current_password: currentPassword,
      })
      .pipe(
        kitErrors,
        map((enrollment) => ({ secret: enrollment.secret, otpauthUrl: enrollment.otpauth_url })),
      )
  }

  confirm(code: string): Observable<BackupCodesResult> {
    return this.http
      .post<{ backup_codes: string[] }>('/api/me/mfa/totp/confirm', { code })
      .pipe(kitErrors, map(toCodes))
  }

  regenerate(currentPassword: string): Observable<BackupCodesResult> {
    return this.http
      .post<{ backup_codes: string[] }>('/api/me/mfa/backup-codes/regenerate', {
        current_password: currentPassword,
      })
      .pipe(kitErrors, map(toCodes))
  }

  disable(currentPassword: string): Observable<void> {
    return this.http
      .delete<void>('/api/me/mfa/totp', { body: { current_password: currentPassword } })
      .pipe(kitErrors)
  }

  startPasskeyRegistration(currentPassword: string): Observable<PasskeyChallenge> {
    return this.http
      .post<{ challenge_id: string; public_key: unknown }>('/api/me/mfa/passkey/register/start', {
        current_password: currentPassword,
      })
      .pipe(
        kitErrors,
        map((start) => ({ challengeId: start.challenge_id, publicKey: start.public_key })),
      )
  }

  finishPasskeyRegistration(
    challengeId: string,
    credential: unknown,
    name: string,
  ): Observable<Passkey> {
    return this.http
      .post<PasskeyRow>('/api/me/mfa/passkey/register/finish', {
        challenge_id: challengeId,
        credential,
        name,
      })
      .pipe(kitErrors, map(toPasskey))
  }

  deletePasskey(id: string, currentPassword: string): Observable<void> {
    return this.http
      .delete<void>(apiPath`/api/me/mfa/passkey/${id}`, {
        body: { current_password: currentPassword },
      })
      .pipe(kitErrors)
  }
}
