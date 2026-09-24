import { Injectable, inject } from '@angular/core'
import { Observable, tap } from 'rxjs'
import {
  IdentityProviderSummary,
  LdapIdentityProviderInput,
  OidcIdentityProviderInput,
  OrganizationSummary,
} from '../domain/organization.entity'
import { ORGANIZATIONS_PORT } from './organizations.port'
import { TokenScopedCache } from '../../shared/token-scoped-cache'

@Injectable({ providedIn: 'root' })
export class OrganizationsService {
  private readonly port = inject(ORGANIZATIONS_PORT)

  private readonly listCache = new TokenScopedCache<OrganizationSummary[]>(() => this.port.list())

  list(options?: { forceRefresh?: boolean }): Observable<OrganizationSummary[]> {
    return this.listCache.get(options?.forceRefresh)
  }

  get(id: string): Observable<OrganizationSummary> {
    return this.port.get(id)
  }

  create(slug: string, displayName: string): Observable<OrganizationSummary> {
    return this.port.create(slug, displayName).pipe(tap(() => this.listCache.clear()))
  }

  getIdentityProvider(id: string): Observable<IdentityProviderSummary> {
    return this.port.getIdentityProvider(id)
  }

  setLdapIdentityProvider(id: string, config: LdapIdentityProviderInput): Observable<void> {
    return this.port.setLdapIdentityProvider(id, config).pipe(tap(() => this.listCache.clear()))
  }

  setOidcIdentityProvider(id: string, config: OidcIdentityProviderInput): Observable<void> {
    return this.port.setOidcIdentityProvider(id, config).pipe(tap(() => this.listCache.clear()))
  }

  clearIdentityProvider(id: string): Observable<void> {
    return this.port.clearIdentityProvider(id).pipe(tap(() => this.listCache.clear()))
  }
}
