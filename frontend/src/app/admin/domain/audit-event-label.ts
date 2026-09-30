import { t } from '../../shared/i18n/translator'
import { AuditEntry } from './audit.entity'

type Payload = Record<string, unknown>

function isRecord(raw: unknown): raw is Payload {
  return typeof raw === 'object' && raw !== null && !Array.isArray(raw)
}

/** The payload as an object; stored events of any age can carry `null` or something else. */
export function auditPayload(entry: AuditEntry): Payload {
  return isRecord(entry.payload) ? entry.payload : {}
}

const EVENT_TYPES = new Set([
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
  'UserInvited',
  'UserActivated',
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
  'BackupCodesRegenerated',
  'InvitationResent',
  'QuotaSet',
  'RetentionPolicySet',
  'LoginThrottleCleared',
  'Created',
  'Renamed',
  'RemoteUrlChanged',
  'GroupMemberAdded',
  'GroupMemberRemoved',
  'VisibilityChanged',
  'Deleted',
  'Granted',
  'Revoked',
  'PackagePushed',
  'PackageVersionUnpublished',
  'PackageDeleted',
  'PackageVersionDeprecated',
  'DistTagChanged',
  'ImagePushed',
  'ManifestDeleted',
])

const LOGIN_METHODS = new Set(['password', 'ldap', 'oidc'])

const MFA_METHODS = new Set(['totp', 'backup_code', 'passkey'])

const SETTINGS = new Set([
  'max_login_attempts',
  'login_attempt_window_seconds',
  'session_ttl_hours',
  'registration_enabled',
  'seo_indexing_enabled',
])

/** The translated name of an event, or the raw type for the ones without a label. */
export function auditEventLabel(eventType: string): string {
  return translated(EVENT_TYPES, 'events', eventType) ?? eventType
}

// Known names only (a Set, not an object lookup), so "constructor" is not a label.
function translated(names: Set<string>, group: string, key: string): string | undefined {
  return names.has(key) ? t(`admin.audit.${group}.${key}`) : undefined
}

/** One line of context for an entry; empty for the events that carry none. */
export function auditEventDetails(entry: AuditEntry): string {
  const payload = auditPayload(entry)
  switch (entry.event_type) {
    case 'LoginSucceeded': {
      const method = mapped(LOGIN_METHODS, 'loginMethods', payload['method'])
      const second = mapped(MFA_METHODS, 'mfaMethods', payload['second_factor'])
      return second
        ? t('admin.audit.details.viaSecond', { method, second })
        : t('admin.audit.details.via', { method })
    }
    case 'MfaEnabled':
    case 'MfaDisabled':
      return t('admin.audit.details.method', {
        method: mapped(MFA_METHODS, 'mfaMethods', payload['method']),
      })
    case 'ApiTokenCreated':
      return t('admin.audit.details.label', { label: text(payload['label']) })
    case 'UserInvited': {
      if (typeof payload['username'] !== 'string') {
        return ''
      }
      const roles = [
        payload['is_super_admin'] === true ? t('admin.audit.details.superAdmin') : null,
        payload['is_organization_admin'] === true
          ? t('admin.audit.details.organizationAdmin')
          : null,
      ].filter((role) => role !== null)
      return roles.length > 0
        ? `${text(payload['username'])} (${roles.join(', ')})`
        : text(payload['username'])
    }
    case 'UserDeleted':
      return text(payload['username'], '')
    case 'LoginThrottleCleared':
      return typeof payload['username'] === 'string'
        ? t('admin.audit.details.user', { username: payload['username'] })
        : ''
    case 'OrganizationCreated':
      return `${text(payload['display_name'])} (${text(payload['slug'])})`
    case 'IdentityProviderSet': {
      const after = provider(payload['after'])
      const before = payload['before'] ? provider(payload['before']) : t('admin.audit.details.none')
      const secret =
        payload['secret_changed'] === true ? t('admin.audit.details.secretChanged') : ''
      return `${before} → ${after}${secret}`
    }
    case 'IdentityProviderCleared':
      return payload['before']
        ? t('admin.audit.details.previous', { provider: provider(payload['before']) })
        : ''
    case 'SmtpSettingsChanged': {
      const after = asRecord(payload['after'])
      const secret =
        payload['password_changed'] === true ? t('admin.audit.details.passwordChanged') : ''
      return `${text(after['host'])}:${text(after['port'])}${secret}`
    }
    case 'SystemSettingsChanged': {
      const changes = Array.isArray(payload['changes']) ? payload['changes'].filter(isRecord) : []
      return changes
        .map(
          (c) =>
            `${translated(SETTINGS, 'settings', String(c['setting'])) ?? text(c['setting'])} : ${value(c['before'])} → ${value(c['after'])}`,
        )
        .join(' ; ')
    }
    case 'BrandingChanged': {
      const asset = payload['asset'] === 'favicon' ? 'favicon' : 'logo'
      return t(
        payload['cleared'] === true
          ? `admin.audit.details.${asset}Removed`
          : `admin.audit.details.${asset}Replaced`,
      )
    }
    case 'ConfigurationExported':
      return t('admin.audit.details.counts', {
        users: text(payload['users'], '0'),
        repositories: text(payload['repositories'], '0'),
        permissions: text(payload['permissions'], '0'),
      })
    case 'ConfigurationImported': {
      const created = t('admin.audit.details.counts', {
        users: text(payload['users_created'], '0'),
        repositories: text(payload['repositories_created'], '0'),
        permissions: text(payload['permissions_granted'], '0'),
      })
      const failures = Number(payload['failures'] ?? 0)
      return failures > 0 ? t('admin.audit.details.withFailures', { created, failures }) : created
    }
    case 'QuotaSet':
      return typeof payload['quota_bytes'] === 'number'
        ? t('admin.audit.details.quotaMb', {
            count: Math.round(payload['quota_bytes'] / (1024 * 1024)),
          })
        : t('admin.audit.details.unlimited')
    case 'RetentionPolicySet':
      return typeof payload['keep_last_n_versions'] === 'number'
        ? t('admin.audit.details.keepLast', { count: payload['keep_last_n_versions'] })
        : t('admin.audit.details.disabled')
    default:
      return ''
  }
}

function asRecord(raw: unknown): Payload {
  return isRecord(raw) ? raw : {}
}

function text(raw: unknown, fallback = '?'): string {
  return typeof raw === 'string' || typeof raw === 'number' ? String(raw) : fallback
}

function mapped(names: Set<string>, group: string, raw: unknown): string {
  return typeof raw === 'string' ? (translated(names, group, raw) ?? raw) : ''
}

function value(raw: unknown): string {
  if (typeof raw === 'boolean') {
    return raw ? t('admin.audit.details.yes') : t('admin.audit.details.no')
  }
  return text(raw)
}

function provider(raw: unknown): string {
  const summary = asRecord(raw)
  return summary['provider'] === 'ldap'
    ? `LDAP ${text(summary['server_url'])}`
    : t('admin.audit.details.oidcProvider', {
        issuer: text(summary['issuer_url']),
        client: text(summary['client_id']),
      })
}
