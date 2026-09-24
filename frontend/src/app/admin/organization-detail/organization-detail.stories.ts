import { signal } from '@angular/core'
import { HttpErrorResponse } from '@angular/common/http'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { NEVER, of, throwError } from 'rxjs'
import { OrganizationDetail } from './organization-detail'
import { OrganizationsService } from '../application/organizations.service'
import { OrganizationMembersService } from '../application/organization-members.service'
import { PageTitleService } from '../../shell/page-title.service'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'
import type {
  IdentityProviderSummary,
  LdapIdentityProvider,
  OidcIdentityProvider,
  OrganizationSummary,
} from '../domain/organization.entity'

const ACME: OrganizationSummary = {
  id: 'org-acme',
  slug: 'acme',
  display_name: 'Acme Corp',
  is_public: false,
}

const LDAP: LdapIdentityProvider = {
  type: 'ldap',
  server_url: 'ldaps://dc.corp.example:636',
  bind_dn: 'cn=service,dc=corp,dc=example',
  bind_password_set: true,
  user_search_base: 'ou=people,dc=corp,dc=example',
  user_search_filter: '(uid=*)',
  email_attribute: 'mail',
}

const OIDC: OidcIdentityProvider = {
  type: 'oidc',
  issuer_url: 'https://accounts.example.com',
  client_id: 'artiferris',
  client_secret_set: true,
}

function fakeOrgs(
  provider: IdentityProviderSummary = { type: null },
  overrides: Partial<OrganizationsService> = {},
): Partial<OrganizationsService> {
  return {
    get: fn(() => of(ACME)),
    getIdentityProvider: fn(() => of(provider)),
    setLdapIdentityProvider: fn(() => of(undefined)),
    setOidcIdentityProvider: fn(() => of(undefined)),
    clearIdentityProvider: fn(() => of(undefined)),
    ...overrides,
  }
}

const toast = { success: fn(), error: fn() }
const pageTitle = { title: signal('') }

/** Stands in for the confirmation dialog the component opens before clearing the provider. */
function fakeConfirm(answer = true) {
  return { ask: fn(() => Promise.resolve(answer)) }
}

function withOrgs(
  orgs: Partial<OrganizationsService>,
  confirm: ReturnType<typeof fakeConfirm> = fakeConfirm(),
) {
  return moduleMetadata({
    providers: [
      { provide: OrganizationsService, useValue: orgs },
      { provide: ConfirmService, useValue: confirm },
    ],
  })
}

async function waitForLoaded(canvas: ReturnType<typeof within>) {
  await waitFor(() =>
    expect(canvas.getByRole('heading', { name: 'Acme Corp' })).toBeInTheDocument(),
  )
}

async function chooseProviderType(canvas: ReturnType<typeof within>, label: 'LDAP' | 'OIDC') {
  await userEvent.click(canvas.getByRole('combobox', { name: "Type de fournisseur d'identité" }))
  await userEvent.click(await canvas.findByRole('option', { name: label }))
}

async function fillLdapForm(canvas: ReturnType<typeof within>) {
  await userEvent.type(canvas.getByLabelText('URL du serveur'), 'ldaps://dc.acme.test:636')
  await userEvent.type(canvas.getByLabelText('DN du compte de service'), 'cn=svc,dc=acme,dc=test')
  await userEvent.type(canvas.getByLabelText('Mot de passe du compte de service'), 'hunter2')
  await userEvent.type(canvas.getByLabelText('Base de recherche'), 'ou=people,dc=acme,dc=test')
  await userEvent.type(canvas.getByLabelText('Filtre de recherche'), '(mail=*)')
  await userEvent.type(canvas.getByLabelText('Attribut e-mail'), 'mail')
}

const meta: Meta<OrganizationDetail> = {
  title: 'Admin/OrganizationDetail',
  component: OrganizationDetail,
  args: { organizationId: 'org-acme' },
  beforeEach: () => {
    toast.success.mockClear()
    toast.error.mockClear()
    pageTitle.title.set('')
  },
  decorators: [
    moduleMetadata({
      providers: [
        { provide: OrganizationsService, useValue: fakeOrgs() },
        {
          provide: OrganizationMembersService,
          useValue: {
            list: () =>
              of([
                {
                  id: 'u-alice',
                  username: 'alice',
                  email: 'alice@acme.test',
                  is_organization_admin: true,
                  invitation_pending: false,
                },
              ]),
          },
        },
        { provide: PageTitleService, useValue: pageTitle },
        { provide: ToastService, useValue: toast },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<OrganizationDetail>

/** No identity provider: the organization uses local accounts, so there's nothing to revert. */
export const LocalAccounts: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForLoaded(canvas)
    expect(canvas.getByText('Sous-domaine : acme')).toBeInTheDocument()
    expect(canvas.getByText('Cette organisation utilise des comptes locaux.')).toBeInTheDocument()
    expect(canvas.getByLabelText('URL du serveur')).toHaveValue('')
    expect(
      canvas.queryByRole('button', { name: 'Revenir aux comptes locaux' }),
    ).not.toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Enregistrer' })).toBeDisabled()
    // The members list is embedded.
    expect(await canvas.findByText('alice')).toBeInTheDocument()
    expect(pageTitle.title()).toBe('Acme Corp')
  },
}

export const Loading: Story = {
  decorators: [withOrgs(fakeOrgs({ type: null }, { get: () => NEVER }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('Chargement…')).toBeInTheDocument())
    expect(canvas.queryByRole('heading', { name: 'Acme Corp' })).not.toBeInTheDocument()
  },
}

export const LoadFailed: Story = {
  decorators: [
    withOrgs(fakeOrgs({ type: null }, { get: () => throwError(() => new Error('boom')) })),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('alert')).toHaveTextContent(
      "Échec du chargement de l'organisation.",
    )
    expect(canvas.queryByRole('button', { name: 'Enregistrer' })).not.toBeInTheDocument()
  },
}

/** When the provider lookup fails there is no form to save defaults over the real configuration. */
export const IdentityProviderLookupFailed: Story = {
  decorators: [
    withOrgs(
      fakeOrgs({ type: null }, { getIdentityProvider: () => throwError(() => new Error('boom')) }),
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('alert')).toHaveTextContent(
      "Échec du chargement de l'organisation.",
    )
    expect(canvas.getByRole('button', { name: 'Réessayer' })).toBeInTheDocument()
    expect(canvas.queryByRole('button', { name: 'Enregistrer' })).not.toBeInTheDocument()
    expect(canvas.queryByText(/comptes locaux/)).not.toBeInTheDocument()
  },
}

/** With a stored bind password, leaving the field blank keeps it, so the form is already saveable. */
export const LdapConfigured: Story = {
  decorators: [withOrgs(fakeOrgs(LDAP))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForLoaded(canvas)
    await waitFor(() =>
      expect(
        canvas.getByText('LDAP est actuellement configuré pour cette organisation.'),
      ).toBeInTheDocument(),
    )
    expect(canvas.getByLabelText('URL du serveur')).toHaveValue('ldaps://dc.corp.example:636')
    expect(canvas.getByLabelText('Mot de passe du compte de service')).toHaveValue('')
    expect(
      canvas.getByPlaceholderText('Laisser vide pour conserver le mot de passe actuel'),
    ).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Enregistrer' })).toBeEnabled()
    expect(canvas.getByRole('button', { name: 'Revenir aux comptes locaux' })).toBeInTheDocument()
  },
}

export const OidcConfigured: Story = {
  decorators: [withOrgs(fakeOrgs(OIDC))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForLoaded(canvas)
    await waitFor(() =>
      expect(
        canvas.getByText('OIDC est actuellement configuré pour cette organisation.'),
      ).toBeInTheDocument(),
    )
    expect(canvas.getByLabelText("URL de l'émetteur (issuer)")).toHaveValue(
      'https://accounts.example.com',
    )
    expect(canvas.getByLabelText('Client ID')).toHaveValue('artiferris')
    expect(
      canvas.getByPlaceholderText('Laisser vide pour conserver le secret actuel'),
    ).toBeInTheDocument()
    expect(canvas.queryByLabelText('URL du serveur')).not.toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Enregistrer' })).toBeEnabled()
  },
}

/** Switching the provider type swaps the LDAP fields for the OIDC ones. */
export const SwitchingToOidc: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForLoaded(canvas)
    expect(canvas.getByLabelText('URL du serveur')).toBeInTheDocument()

    await chooseProviderType(canvas, 'OIDC')
    expect(await canvas.findByLabelText("URL de l'émetteur (issuer)")).toBeInTheDocument()
    expect(canvas.queryByLabelText('URL du serveur')).not.toBeInTheDocument()
  },
}

const savingLdap = fakeOrgs()
export const SavingAnLdapProvider: Story = {
  decorators: [withOrgs(savingLdap)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForLoaded(canvas)
    await fillLdapForm(canvas)
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(savingLdap.setLdapIdentityProvider).toHaveBeenCalledWith('org-acme', {
        server_url: 'ldaps://dc.acme.test:636',
        bind_dn: 'cn=svc,dc=acme,dc=test',
        bind_password: 'hunter2',
        user_search_base: 'ou=people,dc=acme,dc=test',
        user_search_filter: '(mail=*)',
        email_attribute: 'mail',
      }),
    )
    expect(toast.success).toHaveBeenCalledWith('Configuration enregistrée.')
    // Now configured: the password field is emptied and the revert action appears.
    expect(
      await canvas.findByRole('button', { name: 'Revenir aux comptes locaux' }),
    ).toBeInTheDocument()
    expect(canvas.getByLabelText('Mot de passe du compte de service')).toHaveValue('')
  },
}

const keepingSecret = fakeOrgs(OIDC)
export const UpdatingOidcWithoutChangingTheSecret: Story = {
  decorators: [withOrgs(keepingSecret)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByLabelText('Client ID')).toHaveValue('artiferris'))
    const clientId = canvas.getByLabelText('Client ID')
    await userEvent.clear(clientId)
    await userEvent.type(clientId, 'artiferris-v2')
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(keepingSecret.setOidcIdentityProvider).toHaveBeenCalledWith('org-acme', {
        issuer_url: 'https://accounts.example.com',
        client_id: 'artiferris-v2',
        client_secret: undefined,
      }),
    )
    expect(toast.success).toHaveBeenCalledWith('Configuration enregistrée.')
  },
}

const firstOidc = fakeOrgs()
/** With no secret stored yet, the OIDC form needs one before it can be saved. */
export const ConfiguringOidcFromScratch: Story = {
  decorators: [withOrgs(firstOidc)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForLoaded(canvas)
    await chooseProviderType(canvas, 'OIDC')
    await userEvent.type(
      await canvas.findByLabelText("URL de l'émetteur (issuer)"),
      'https://sso.acme.test',
    )
    await userEvent.type(canvas.getByLabelText('Client ID'), 'acme-app')
    expect(canvas.getByRole('button', { name: 'Enregistrer' })).toBeDisabled()

    await userEvent.type(canvas.getByLabelText('Client secret'), 's3cret')
    await waitFor(() => expect(canvas.getByRole('button', { name: 'Enregistrer' })).toBeEnabled())
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(firstOidc.setOidcIdentityProvider).toHaveBeenCalledWith('org-acme', {
        issuer_url: 'https://sso.acme.test',
        client_id: 'acme-app',
        client_secret: 's3cret',
      }),
    )
    expect(firstOidc.setLdapIdentityProvider).not.toHaveBeenCalled()
  },
}

const savingInFlight = fakeOrgs(LDAP, { setLdapIdentityProvider: fn(() => NEVER) })
export const Saving: Story = {
  decorators: [withOrgs(savingInFlight)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() =>
      expect(canvas.getByLabelText('URL du serveur')).toHaveValue('ldaps://dc.corp.example:636'),
    )
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    const button = await canvas.findByRole('button', { name: /Enregistrement…/ })
    await waitFor(() => expect(button).toHaveAttribute('aria-busy', 'true'))
    expect(button).toBeDisabled()
  },
}

export const SaveFailed: Story = {
  decorators: [
    withOrgs(
      fakeOrgs(LDAP, { setLdapIdentityProvider: fn(() => throwError(() => new Error('boom'))) }),
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() =>
      expect(canvas.getByLabelText('URL du serveur')).toHaveValue('ldaps://dc.corp.example:636'),
    )
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith('Échec de la mise à jour de la configuration.'),
    )
    expect(toast.success).not.toHaveBeenCalled()
    expect(canvas.getByRole('button', { name: 'Enregistrer' })).toBeEnabled()
  },
}

const keptSecretRefused = fakeOrgs(LDAP, {
  setLdapIdentityProvider: fn(() =>
    throwError(
      () =>
        new HttpErrorResponse({
          status: 400,
          error: {
            error: 're-enter the bind password when changing the server URL or the bind DN',
          },
        }),
    ),
  ),
})

/** Changing the server without retyping the secret is refused: the server's own message is shown. */
export const KeptSecretRefusedAfterChangingTheServer: Story = {
  decorators: [withOrgs(keptSecretRefused)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() =>
      expect(canvas.getByLabelText('URL du serveur')).toHaveValue('ldaps://dc.corp.example:636'),
    )
    await userEvent.clear(canvas.getByLabelText('URL du serveur'))
    await userEvent.type(canvas.getByLabelText('URL du serveur'), 'ldaps://other.example:636')
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith(
        're-enter the bind password when changing the server URL or the bind DN',
      ),
    )
    expect(toast.success).not.toHaveBeenCalled()
  },
}

/** A 409 on save means the stored secret is unreadable: the toast says so and the warning appears. */
export const SaveAnswersSecretUnreadable: Story = {
  decorators: [
    withOrgs(
      fakeOrgs(LDAP, {
        setLdapIdentityProvider: fn(() =>
          throwError(() => new HttpErrorResponse({ status: 409, error: { error: 'unreadable' } })),
        ),
      }),
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() =>
      expect(canvas.getByLabelText('URL du serveur')).toHaveValue('ldaps://dc.corp.example:636'),
    )
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith(
        'Le secret enregistré est illisible : saisissez-le à nouveau',
      ),
    )
    expect(await canvas.findByRole('alert')).toHaveTextContent('illisible')
  },
}

/** The stored secret cannot be decrypted by this server: a warning replaces the "local accounts" line and the secret must be typed again. */
export const StoredSecretUnreadable: Story = {
  decorators: [
    withOrgs(
      fakeOrgs({
        type: null,
        secret_unreadable: true,
        error: "the stored secret cannot be read with this server's SECRETS_ENCRYPTION_KEY",
      }),
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForLoaded(canvas)
    expect(await canvas.findByRole('alert')).toHaveTextContent(
      "Le secret enregistré pour le fournisseur d'identité est illisible",
    )
    expect(canvas.queryByText('Cette organisation utilise des comptes locaux.')).toBeNull()
    expect(canvas.getByRole('button', { name: 'Enregistrer' })).toBeDisabled()
    expect(canvas.getByRole('button', { name: 'Revenir aux comptes locaux' })).toBeInTheDocument()
  },
}

/** An OIDC issuer must be https: the field says so and saving stays disabled. */
export const HttpIssuerRefused: Story = {
  decorators: [withOrgs(fakeOrgs())],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForLoaded(canvas)
    await chooseProviderType(canvas, 'OIDC')
    await userEvent.type(
      await canvas.findByLabelText("URL de l'émetteur (issuer)"),
      'http://sso.acme.test',
    )
    await userEvent.type(canvas.getByLabelText('Client ID'), 'acme-app')
    await userEvent.type(canvas.getByLabelText('Client secret'), 's3cret')

    expect(
      await canvas.findByText("L'URL de l'émetteur doit commencer par https://."),
    ).toBeVisible()
    expect(canvas.getByRole('button', { name: 'Enregistrer' })).toBeDisabled()
  },
}

const clearing = fakeOrgs(LDAP)
const confirmClearing = fakeConfirm(true)
export const RevertingToLocalAccounts: Story = {
  decorators: [withOrgs(clearing, confirmClearing)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Revenir aux comptes locaux' }))

    await waitFor(() =>
      expect(confirmClearing.ask).toHaveBeenCalledWith(
        expect.objectContaining({
          message: 'Revenir aux comptes locaux pour cette organisation ?',
        }),
      ),
    )
    await waitFor(() => expect(clearing.clearIdentityProvider).toHaveBeenCalledWith('org-acme'))
    expect(toast.success).toHaveBeenCalledWith('Retour aux comptes locaux effectué.')
    await waitFor(() =>
      expect(
        canvas.getByText('Cette organisation utilise des comptes locaux.'),
      ).toBeInTheDocument(),
    )
    expect(canvas.getByLabelText('URL du serveur')).toHaveValue('')
    expect(
      canvas.queryByRole('button', { name: 'Revenir aux comptes locaux' }),
    ).not.toBeInTheDocument()
  },
}

const declinedClear = fakeOrgs(LDAP)
const confirmDeclined = fakeConfirm(false)
export const RevertDeclined: Story = {
  decorators: [withOrgs(declinedClear, confirmDeclined)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Revenir aux comptes locaux' }))

    await waitFor(() => expect(confirmDeclined.ask).toHaveBeenCalled())
    expect(declinedClear.clearIdentityProvider).not.toHaveBeenCalled()
    expect(canvas.getByLabelText('URL du serveur')).toHaveValue('ldaps://dc.corp.example:636')
  },
}

export const RevertFailed: Story = {
  decorators: [
    withOrgs(
      fakeOrgs(LDAP, { clearIdentityProvider: fn(() => throwError(() => new Error('boom'))) }),
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Revenir aux comptes locaux' }))

    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith('Échec de la suppression de la configuration.'),
    )
    expect(toast.success).not.toHaveBeenCalled()
    expect(canvas.getByLabelText('URL du serveur')).toHaveValue('ldaps://dc.corp.example:636')
    expect(canvas.getByRole('button', { name: 'Revenir aux comptes locaux' })).toBeEnabled()
  },
}
