import { auditEventDetails, auditEventLabel } from './audit-event-label'
import { AuditEntry } from './audit.entity'

const entry = (event_type: string, payload: unknown): AuditEntry => ({
  aggregate_type: 'Admin',
  aggregate_id: 'a1',
  event_type,
  payload,
  occurred_at: '2026-01-01T00:00:00Z',
  actor_id: 'u1',
})

describe('audit event labels', () => {
  // Every event_type the backend can store (enums in crates/artiferris-domain). Add a variant
  // there, add it here.
  const BACKEND_EVENT_TYPES = [
    // SecurityEvent
    'LoginFailed',
    'AccessDenied',
    'PasswordChangeFailed',
    'MfaVerificationFailed',
    'PasskeyVerificationFailed',
    'OidcLoginFailed',
    'DockerTokenFailed',
    'LoginSucceeded',
    'PasswordChanged',
    'MfaEnabled',
    'MfaDisabled',
    'PasskeyAdded',
    'PasskeyDeleted',
    'ApiTokenCreated',
    'ApiTokenRevoked',
    'SessionsRevoked',
    'BackupCodesRegenerated',
    // AdminAuditEvent
    'UserInvited',
    'UserActivated',
    'InvitationResent',
    'UserDeleted',
    'SuperAdminGranted',
    'SuperAdminRevoked',
    'OrganizationAdminGranted',
    'OrganizationAdminRevoked',
    'OrganizationCreated',
    'IdentityProviderSet',
    'IdentityProviderCleared',
    'SmtpSettingsChanged',
    'SystemSettingsChanged',
    'BrandingChanged',
    'ConfigurationExported',
    'ConfigurationImported',
    'LoginThrottleCleared',
    // PackageRepositoryEvent
    'Created',
    'Renamed',
    'RemoteUrlChanged',
    'GroupMemberAdded',
    'GroupMemberRemoved',
    'QuotaSet',
    'RetentionPolicySet',
    'VisibilityChanged',
    'Deleted',
    // PermissionEvent
    'Granted',
    'Revoked',
    // NpmPackageEvent
    'PackagePushed',
    'PackageVersionUnpublished',
    'PackageDeleted',
    'PackageVersionDeprecated',
    'DistTagChanged',
    // DockerRegistryEvent
    'ImagePushed',
    'ManifestDeleted',
  ]

  it.each(BACKEND_EVENT_TYPES)('names %s in French', (kind) => {
    expect(auditEventLabel(kind)).not.toBe(kind)
  })

  it('labels the admin unlock of a login throttle', () => {
    expect(auditEventLabel('LoginThrottleCleared')).toBe('Verrouillage de connexion levé')
  })

  it('names the user whose login lock was lifted, whether or not the account exists', () => {
    expect(
      auditEventDetails(
        entry('LoginThrottleCleared', { organization_id: 'org-1', username: 'alice' }),
      ),
    ).toBe('Utilisateur alice')
    expect(
      auditEventDetails(
        entry('LoginThrottleCleared', { organization_id: null, username: 'ghost' }),
      ),
    ).toBe('Utilisateur ghost')
    expect(auditEventDetails(entry('LoginThrottleCleared', null))).toBe('')
  })

  it('names the invited address, or the username on events recorded before invitees chose their own', () => {
    expect(
      auditEventDetails(
        entry('UserInvited', { email: 'alice@example.com', is_organization_admin: false }),
      ),
    ).toBe('alice@example.com')
    expect(auditEventDetails(entry('UserInvited', { username: 'alice' }))).toBe('alice')
  })

  it('never resolves a type to an inherited object member', () => {
    expect(auditEventLabel('constructor')).toBe('constructor')
    expect(auditEventLabel('toString')).toBe('toString')
    expect(
      auditEventDetails(entry('LoginSucceeded', { method: 'constructor', second_factor: null })),
    ).toBe('Via constructor')
  })

  it('falls back to the raw type for an event it does not know', () => {
    expect(auditEventLabel('SomethingNew')).toBe('SomethingNew')
    expect(auditEventDetails(entry('SomethingNew', { package_name: 'left-pad' }))).toBe('')
  })

  it('describes a login by method and second factor', () => {
    expect(
      auditEventDetails(entry('LoginSucceeded', { method: 'password', second_factor: 'passkey' })),
    ).toBe("Via mot de passe + clé d'accès")
    expect(
      auditEventDetails(entry('LoginSucceeded', { method: 'oidc', second_factor: null })),
    ).toBe('Via SSO (OIDC)')
  })

  it('shows an identity provider swap as before and after, and whether the secret changed', () => {
    const details = auditEventDetails(
      entry('IdentityProviderSet', {
        before: { provider: 'oidc', issuer_url: 'https://idp.example', client_id: 'a' },
        after: { provider: 'oidc', issuer_url: 'https://evil.example', client_id: 'b' },
        secret_changed: true,
      }),
    )
    expect(details).toBe(
      'OIDC https://idp.example (client a) → OIDC https://evil.example (client b), secret modifié',
    )
  })

  it('lists changed system settings with their French names', () => {
    const details = auditEventDetails(
      entry('SystemSettingsChanged', {
        changes: [
          { setting: 'registration_enabled', before: true, after: false },
          { setting: 'max_login_attempts', before: 10, after: 3 },
        ],
      }),
    )
    expect(details).toBe('Inscription ouverte : oui → non ; Tentatives de connexion max. : 10 → 3')
  })

  it('summarises configuration export and import by counts', () => {
    expect(
      auditEventDetails(
        entry('ConfigurationExported', { users: 4, repositories: 2, permissions: 9 }),
      ),
    ).toBe('4 utilisateurs, 2 dépôts, 9 permissions')
    expect(
      auditEventDetails(
        entry('ConfigurationImported', {
          users_created: 4,
          repositories_created: 2,
          permissions_granted: 9,
          failures: 1,
        }),
      ),
    ).toBe('4 utilisateurs, 2 dépôts, 9 permissions ; 1 échec(s)')
  })

  it('does not choke on an unexpected payload', () => {
    expect(() => auditEventDetails(entry('SystemSettingsChanged', null))).not.toThrow()
    expect(() => auditEventDetails(entry('IdentityProviderSet', 'oops'))).not.toThrow()
  })

  it('skips a null element of a settings change list instead of throwing', () => {
    expect(
      auditEventDetails(
        entry('SystemSettingsChanged', {
          changes: [null, { setting: 'session_ttl_hours', before: 8, after: 12 }, 'x', 3],
        }),
      ),
    ).toBe('Durée de session (h) : 8 → 12')
  })

  it.each([null, undefined, 'text', 42, []])('does not throw on a %j payload', (payload) => {
    for (const kind of BACKEND_EVENT_TYPES) {
      expect(() => auditEventDetails(entry(kind, payload)), kind).not.toThrow()
    }
    expect(auditEventDetails(entry('SmtpSettingsChanged', payload))).toBe('?:?')
  })

  it('tolerates null before/after summaries', () => {
    expect(
      auditEventDetails(entry('IdentityProviderSet', { before: null, after: null })),
    ).toContain('aucun → OIDC')
    expect(auditEventDetails(entry('SmtpSettingsChanged', { after: null }))).toBe('?:?')
  })
})
