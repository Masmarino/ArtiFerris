import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  signal,
} from '@angular/core'
import { FormsModule } from '@angular/forms'
import { forkJoin } from 'rxjs'
import {
  Alert,
  Button,
  Card,
  GbtInput,
  Select,
  SelectOption,
  Spinner,
  Tooltip,
} from '@masmarino/gabarit'
import { PageTitleService } from '../../shell/page-title.service'
import { OrganizationsService } from '../application/organizations.service'
import { IdentityProviderSummary, OrganizationSummary } from '../domain/organization.entity'
import { OrganizationMembers } from '../organization-members/organization-members'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'
import { isSecretUnreadable, secretFormFailureMessage } from '../../shared/api-error'

const PROVIDER_TYPE_OPTIONS: SelectOption<'ldap' | 'oidc'>[] = [
  { value: 'ldap', label: 'LDAP' },
  { value: 'oidc', label: 'OIDC' },
]

@Component({
  selector: 'app-organization-detail',
  standalone: true,
  imports: [
    Alert,
    Card,
    GbtInput,
    Button,
    Select,
    FormsModule,
    OrganizationMembers,
    Spinner,
    Tooltip,
  ],
  templateUrl: './organization-detail.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class OrganizationDetail {
  private readonly organizationsService = inject(OrganizationsService)
  private readonly pageTitle = inject(PageTitleService)
  private readonly confirmService = inject(ConfirmService)
  private readonly toastService = inject(ToastService)

  readonly organizationId = input.required<string>()

  readonly organization = signal<OrganizationSummary | null>(null)
  readonly loading = signal(true)

  readonly providerTypeOptions = PROVIDER_TYPE_OPTIONS
  readonly selectedProviderType = signal<'ldap' | 'oidc'>('ldap')

  readonly identityProviderConfigured = signal(false)
  /** The stored secret cannot be decrypted by this server: the provider must be configured again. */
  readonly secretUnreadable = signal(false)
  readonly serverUrl = signal('')
  readonly bindDn = signal('')
  readonly bindPassword = signal('')
  readonly bindPasswordSet = signal(false)
  readonly userSearchBase = signal('')
  readonly userSearchFilter = signal('')
  readonly emailAttribute = signal('')

  readonly issuerUrl = signal('')
  readonly issuerError = computed(() => {
    const url = this.issuerUrl().trim()
    return url !== '' && !url.toLowerCase().startsWith('https://')
      ? "L'URL de l'émetteur doit commencer par https://."
      : null
  })
  readonly clientId = signal('')
  readonly clientSecret = signal('')
  readonly clientSecretSet = signal(false)

  readonly saving = signal(false)
  readonly errorMessage = signal<string | null>(null)
  readonly clearing = signal(false)

  readonly hasErrors = computed(() => {
    if (this.selectedProviderType() === 'oidc') {
      return (
        this.issuerUrl().trim() === '' ||
        this.issuerError() !== null ||
        this.clientId().trim() === '' ||
        (!this.clientSecretSet() && this.clientSecret().trim() === '')
      )
    }
    return (
      this.serverUrl().trim() === '' ||
      this.bindDn().trim() === '' ||
      this.userSearchBase().trim() === '' ||
      this.userSearchFilter().trim() === '' ||
      this.emailAttribute().trim() === '' ||
      (!this.bindPasswordSet() && this.bindPassword().trim() === '')
    )
  })

  // effect(), not ngOnInit — this component is reused across organizations on the same route.
  constructor() {
    effect(() => {
      this.organizationId()
      this.reload()
    })
  }

  reload(): void {
    const requestedId = this.organizationId()
    // The form shows only once both answers are in, so a failed lookup never leaves defaults to save.
    this.organization.set(null)
    this.resetIdentityProvider()
    this.loading.set(true)
    this.errorMessage.set(null)
    this.saving.set(false)
    this.clearing.set(false)
    forkJoin({
      organization: this.organizationsService.get(requestedId),
      identityProvider: this.organizationsService.getIdentityProvider(requestedId),
    }).subscribe({
      next: ({ organization, identityProvider }) => {
        if (requestedId !== this.organizationId()) {
          return
        }
        this.applyIdentityProvider(identityProvider)
        this.organization.set(organization)
        this.pageTitle.title.set(organization.display_name)
        this.loading.set(false)
      },
      error: () => {
        if (requestedId === this.organizationId()) {
          this.loading.set(false)
          this.errorMessage.set("Échec du chargement de l'organisation.")
        }
      },
    })
  }

  private applyIdentityProvider(config: IdentityProviderSummary): void {
    this.secretUnreadable.set(config.type === null && config.secret_unreadable === true)
    if (config.type === 'ldap') {
      this.identityProviderConfigured.set(true)
      this.selectedProviderType.set('ldap')
      this.serverUrl.set(config.server_url)
      this.bindDn.set(config.bind_dn)
      this.bindPasswordSet.set(config.bind_password_set)
      this.userSearchBase.set(config.user_search_base)
      this.userSearchFilter.set(config.user_search_filter)
      this.emailAttribute.set(config.email_attribute)
    } else if (config.type === 'oidc') {
      this.identityProviderConfigured.set(true)
      this.selectedProviderType.set('oidc')
      this.issuerUrl.set(config.issuer_url)
      this.clientId.set(config.client_id)
      this.clientSecretSet.set(config.client_secret_set)
    }
  }

  private resetIdentityProvider(): void {
    this.identityProviderConfigured.set(false)
    this.secretUnreadable.set(false)
    this.selectedProviderType.set('ldap')
    this.serverUrl.set('')
    this.bindDn.set('')
    this.bindPassword.set('')
    this.bindPasswordSet.set(false)
    this.userSearchBase.set('')
    this.userSearchFilter.set('')
    this.emailAttribute.set('')
    this.issuerUrl.set('')
    this.clientId.set('')
    this.clientSecret.set('')
    this.clientSecretSet.set(false)
  }

  save(): void {
    if (this.hasErrors()) {
      return
    }
    this.saving.set(true)
    const requestedId = this.organizationId()
    const request$ =
      this.selectedProviderType() === 'oidc'
        ? this.organizationsService.setOidcIdentityProvider(requestedId, {
            issuer_url: this.issuerUrl(),
            client_id: this.clientId(),
            client_secret: this.clientSecret().trim() === '' ? undefined : this.clientSecret(),
          })
        : this.organizationsService.setLdapIdentityProvider(requestedId, {
            server_url: this.serverUrl(),
            bind_dn: this.bindDn(),
            bind_password: this.bindPassword().trim() === '' ? undefined : this.bindPassword(),
            user_search_base: this.userSearchBase(),
            user_search_filter: this.userSearchFilter(),
            email_attribute: this.emailAttribute(),
          })
    request$.subscribe({
      next: () => {
        if (requestedId !== this.organizationId()) {
          return
        }
        this.saving.set(false)
        this.identityProviderConfigured.set(true)
        this.secretUnreadable.set(false)
        if (this.selectedProviderType() === 'oidc') {
          this.clientSecretSet.set(true)
          this.clientSecret.set('')
        } else {
          this.bindPasswordSet.set(true)
          this.bindPassword.set('')
        }
        this.toastService.success('Configuration enregistrée.')
      },
      error: (error: unknown) => {
        if (requestedId === this.organizationId()) {
          this.saving.set(false)
          if (isSecretUnreadable(error)) {
            this.secretUnreadable.set(true)
          }
        }
        this.toastService.error(
          secretFormFailureMessage(error, 'Échec de la mise à jour de la configuration.'),
        )
      },
    })
  }

  async clear(): Promise<void> {
    const confirmed = await this.confirmService.ask({
      heading: 'Revenir aux comptes locaux',
      message: 'Revenir aux comptes locaux pour cette organisation ?',
      confirmLabel: 'Revenir',
      danger: true,
    })
    if (!confirmed) return
    this.clearing.set(true)
    const requestedId = this.organizationId()
    this.organizationsService.clearIdentityProvider(requestedId).subscribe({
      next: () => {
        if (requestedId !== this.organizationId()) {
          return
        }
        this.clearing.set(false)
        this.identityProviderConfigured.set(false)
        this.serverUrl.set('')
        this.bindDn.set('')
        this.bindPasswordSet.set(false)
        this.userSearchBase.set('')
        this.userSearchFilter.set('')
        this.emailAttribute.set('')
        this.issuerUrl.set('')
        this.clientId.set('')
        this.clientSecretSet.set(false)
        this.toastService.success('Retour aux comptes locaux effectué.')
      },
      error: () => {
        if (requestedId === this.organizationId()) {
          this.clearing.set(false)
        }
        this.toastService.error('Échec de la suppression de la configuration.')
      },
    })
  }
}
