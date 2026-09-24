import { AuditEntry } from './audit.entity'

type Payload = Record<string, unknown>

function isRecord(raw: unknown): raw is Payload {
  return typeof raw === 'object' && raw !== null && !Array.isArray(raw)
}

/** The payload as an object; stored events of any age can carry `null` or something else. */
export function auditPayload(entry: AuditEntry): Payload {
  return isRecord(entry.payload) ? entry.payload : {}
}

const LABELS: Record<string, string> = {
  LoginFailed: 'Échec de connexion',
  AccessDenied: 'Accès refusé',
  PasswordChangeFailed: 'Échec de changement de mot de passe',
  MfaVerificationFailed: 'Échec de vérification de la double authentification',
  PasskeyVerificationFailed: "Échec de vérification d'une clé d'accès",
  OidcLoginFailed: 'Échec de connexion SSO (OIDC)',
  DockerTokenFailed: "Échec d'obtention d'un jeton Docker",
  LoginSucceeded: 'Connexion réussie',
  PasswordChanged: 'Mot de passe modifié',
  MfaEnabled: 'Double authentification activée',
  MfaDisabled: 'Double authentification désactivée',
  PasskeyAdded: "Clé d'accès ajoutée",
  PasskeyDeleted: "Clé d'accès supprimée",
  ApiTokenCreated: "Jeton d'API créé",
  ApiTokenRevoked: "Jeton d'API révoqué",
  SessionsRevoked: 'Déconnexion de toutes les sessions',
  UserInvited: 'Utilisateur invité',
  UserActivated: 'Compte activé',
  UserDeleted: 'Utilisateur supprimé',
  SuperAdminGranted: 'Super-administrateur accordé',
  SuperAdminRevoked: 'Super-administrateur retiré',
  OrganizationAdminGranted: "Administrateur d'organisation accordé",
  OrganizationAdminRevoked: "Administrateur d'organisation retiré",
  OrganizationCreated: 'Organisation créée',
  IdentityProviderSet: "Fournisseur d'identité modifié",
  IdentityProviderCleared: "Fournisseur d'identité supprimé",
  SmtpSettingsChanged: 'Paramètres SMTP modifiés',
  SystemSettingsChanged: 'Paramètres système modifiés',
  BrandingChanged: 'Image de marque modifiée',
  ConfigurationExported: 'Configuration exportée',
  ConfigurationImported: 'Configuration importée',
  BackupCodesRegenerated: 'Codes de secours régénérés',
  InvitationResent: 'Invitation renvoyée',
  QuotaSet: 'Quota modifié',
  RetentionPolicySet: 'Rétention modifiée',
  LoginThrottleCleared: 'Verrouillage de connexion levé',
  Created: 'Dépôt créé',
  Renamed: 'Dépôt renommé',
  RemoteUrlChanged: 'URL distante modifiée',
  GroupMemberAdded: 'Membre ajouté au groupe',
  GroupMemberRemoved: 'Membre retiré du groupe',
  VisibilityChanged: 'Visibilité modifiée',
  Deleted: 'Dépôt supprimé',
  Granted: 'Accès accordé',
  Revoked: 'Accès retiré',
  PackagePushed: 'Paquet publié',
  PackageVersionUnpublished: 'Version dépubliée',
  PackageDeleted: 'Paquet supprimé',
  PackageVersionDeprecated: 'Version dépréciée',
  DistTagChanged: 'Dist-tag modifié',
  ImagePushed: 'Image publiée',
  ManifestDeleted: 'Manifeste supprimé',
}

const LOGIN_METHODS: Record<string, string> = {
  password: 'mot de passe',
  ldap: 'LDAP',
  oidc: 'SSO (OIDC)',
}

const MFA_METHODS: Record<string, string> = {
  totp: 'application TOTP',
  backup_code: 'code de secours',
  passkey: "clé d'accès",
}

const SETTINGS: Record<string, string> = {
  max_login_attempts: 'Tentatives de connexion max.',
  login_attempt_window_seconds: 'Fenêtre de tentatives (s)',
  session_ttl_hours: 'Durée de session (h)',
  registration_enabled: 'Inscription ouverte',
  seo_indexing_enabled: 'Indexation par les moteurs de recherche',
}

/** The French name of an event, or the raw type for the ones without a label. */
export function auditEventLabel(eventType: string): string {
  return lookup(LABELS, eventType) ?? eventType
}

// Own keys only, so "constructor" is not a label.
function lookup(names: Record<string, string>, key: string): string | undefined {
  return Object.hasOwn(names, key) ? names[key] : undefined
}

/** One line of context for an entry; empty for the events that carry none. */
export function auditEventDetails(entry: AuditEntry): string {
  const payload = auditPayload(entry)
  switch (entry.event_type) {
    case 'LoginSucceeded': {
      const method = mapped(LOGIN_METHODS, payload['method'])
      const second = mapped(MFA_METHODS, payload['second_factor'])
      return second ? `Via ${method} + ${second}` : `Via ${method}`
    }
    case 'MfaEnabled':
    case 'MfaDisabled':
      return `Méthode : ${mapped(MFA_METHODS, payload['method'])}`
    case 'ApiTokenCreated':
      return `Libellé : ${text(payload['label'])}`
    case 'UserInvited': {
      if (typeof payload['username'] !== 'string') {
        return ''
      }
      const roles = [
        payload['is_super_admin'] === true ? 'super-administrateur' : null,
        payload['is_organization_admin'] === true ? "administrateur d'organisation" : null,
      ].filter((role) => role !== null)
      return roles.length > 0
        ? `${text(payload['username'])} (${roles.join(', ')})`
        : text(payload['username'])
    }
    case 'UserDeleted':
      return text(payload['username'], '')
    case 'LoginThrottleCleared':
      return typeof payload['username'] === 'string' ? `Utilisateur ${payload['username']}` : ''
    case 'OrganizationCreated':
      return `${text(payload['display_name'])} (${text(payload['slug'])})`
    case 'IdentityProviderSet': {
      const after = provider(payload['after'])
      const before = payload['before'] ? provider(payload['before']) : 'aucun'
      const secret = payload['secret_changed'] === true ? ', secret modifié' : ''
      return `${before} → ${after}${secret}`
    }
    case 'IdentityProviderCleared':
      return payload['before'] ? `Ancien : ${provider(payload['before'])}` : ''
    case 'SmtpSettingsChanged': {
      const after = asRecord(payload['after'])
      const secret = payload['password_changed'] === true ? ', mot de passe modifié' : ''
      return `${text(after['host'])}:${text(after['port'])}${secret}`
    }
    case 'SystemSettingsChanged': {
      const changes = Array.isArray(payload['changes']) ? payload['changes'].filter(isRecord) : []
      return changes
        .map(
          (c) =>
            `${lookup(SETTINGS, String(c['setting'])) ?? text(c['setting'])} : ${value(c['before'])} → ${value(c['after'])}`,
        )
        .join(' ; ')
    }
    case 'BrandingChanged': {
      const asset = payload['asset'] === 'favicon' ? 'Favicon' : 'Logo'
      return `${asset} ${payload['cleared'] === true ? 'supprimé' : 'remplacé'}`
    }
    case 'ConfigurationExported':
      return `${text(payload['users'], '0')} utilisateurs, ${text(payload['repositories'], '0')} dépôts, ${text(payload['permissions'], '0')} permissions`
    case 'ConfigurationImported': {
      const created = `${text(payload['users_created'], '0')} utilisateurs, ${text(payload['repositories_created'], '0')} dépôts, ${text(payload['permissions_granted'], '0')} permissions`
      const failures = Number(payload['failures'] ?? 0)
      return failures > 0 ? `${created} ; ${failures} échec(s)` : created
    }
    case 'QuotaSet':
      return typeof payload['quota_bytes'] === 'number'
        ? `${Math.round(payload['quota_bytes'] / (1024 * 1024))} Mo`
        : 'Illimité'
    case 'RetentionPolicySet':
      return typeof payload['keep_last_n_versions'] === 'number'
        ? `Garder les ${payload['keep_last_n_versions']} dernières versions`
        : 'Désactivée'
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

function mapped(names: Record<string, string>, raw: unknown): string {
  return typeof raw === 'string' ? (lookup(names, raw) ?? raw) : ''
}

function value(raw: unknown): string {
  if (typeof raw === 'boolean') {
    return raw ? 'oui' : 'non'
  }
  return text(raw)
}

function provider(raw: unknown): string {
  const summary = asRecord(raw)
  return summary['provider'] === 'ldap'
    ? `LDAP ${text(summary['server_url'])}`
    : `OIDC ${text(summary['issuer_url'])} (client ${text(summary['client_id'])})`
}
