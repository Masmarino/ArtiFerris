export const PUBLIC_ORGANIZATION_ID = '00000000-0000-0000-0000-000000000001'

export interface OrganizationSummary {
  id: string
  slug: string
  display_name: string
  is_public: boolean
}

export interface LdapIdentityProvider {
  type: 'ldap'
  server_url: string
  bind_dn: string
  bind_password_set: boolean
  user_search_base: string
  user_search_filter: string
  email_attribute: string
}

export interface OidcIdentityProvider {
  type: 'oidc'
  issuer_url: string
  client_id: string
  client_secret_set: boolean
}

export interface NoIdentityProvider {
  type: null
  secret_unreadable?: boolean
  error?: string
}

export type IdentityProviderSummary =
  LdapIdentityProvider | OidcIdentityProvider | NoIdentityProvider

export interface LdapIdentityProviderInput {
  server_url: string
  bind_dn: string
  /** Empty or omitted keeps the stored password. */
  bind_password?: string
  user_search_base: string
  user_search_filter: string
  email_attribute: string
}

export interface OidcIdentityProviderInput {
  issuer_url: string
  client_id: string
  /** Empty or omitted keeps the stored secret. */
  client_secret?: string
}
