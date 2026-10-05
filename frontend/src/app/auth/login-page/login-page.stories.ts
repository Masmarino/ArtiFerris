import { applicationConfig, type Meta, type StoryObj } from '@storybook/angular-vite'
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { of } from 'rxjs'
import { LoginPage } from './login-page'
import { AuthPort } from '../application/auth.port'
import { fakeAuthPort, mfaPending, refused, withAuthPort } from '../kit/auth-story-helpers'

const page = (port: AuthPort, query: Record<string, string> = {}) =>
  applicationConfig({
    providers: [
      provideRouter([]),
      withAuthPort(port),
      {
        provide: ActivatedRoute,
        useValue: { snapshot: { queryParamMap: convertToParamMap(query) } },
      },
    ],
  })

async function signIn(canvasElement: HTMLElement, username = 'florian', password = 'hunter22') {
  const canvas = within(canvasElement)
  await userEvent.type(await canvas.findByLabelText("Nom d'utilisateur"), username)
  await userEvent.type(canvas.getByLabelText('Mot de passe'), password)
  await userEvent.click(canvas.getByRole('button', { name: 'Se connecter' }))
  return canvas
}

/**
 * The sign-in page: Gabarit's panel on the graphite page, the instance's logo, then the second
 * factor (the challenge, or the mandatory first enrolment). The way to the public packages stays
 * under the panel.
 */
const meta: Meta<LoginPage> = {
  title: 'Auth/LoginPage',
  component: LoginPage,
  parameters: { layout: 'fullscreen' },
  decorators: [page(fakeAuthPort())],
}
export default meta

type Story = StoryObj<LoginPage>

export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByLabelText("Nom d'utilisateur")).toHaveFocus())
    await expect(canvas.getByRole('link', { name: 'Créer un compte' })).toBeVisible()
    await expect(
      canvas.getByRole('link', { name: 'Explorer les paquets publics' }),
    ).toHaveAttribute('href', expect.stringMatching(/\/explorer$/))
  },
}

export const AfterSessionsWereEnded: Story = {
  decorators: [page(fakeAuthPort(), { reason: 'sessions-revoked' })],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      await canvas.findByText('Vos sessions ont été fermées : reconnectez-vous.'),
    ).toBeVisible()
  },
}

const ldapLogin = fn(() => of(mfaPending()))

/** An LDAP organisation signs in through the same form, against its directory. */
export const LdapSso: Story = {
  decorators: [
    page(
      fakeAuthPort({
        getSsoConfig: () => of({ type: 'ldap' as const, registration_enabled: false }),
        loginWithLdap: ldapLogin,
      }),
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = await signIn(canvasElement)
    await waitFor(() => expect(ldapLogin).toHaveBeenCalledWith('florian', 'hunter22'))
    await expect(await canvas.findByLabelText('Code à 6 chiffres')).toBeVisible()
    await expect(canvas.queryByRole('link', { name: 'Créer un compte' })).toBeNull()
  },
}

/** An OIDC organisation signs in at its identity provider: no form, one button. */
export const OidcSso: Story = {
  decorators: [
    page(
      fakeAuthPort({
        getSsoConfig: () => of({ type: 'oidc' as const, registration_enabled: false }),
      }),
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const link = await canvas.findByRole('link', {
      name: "Se connecter avec le fournisseur d'identité",
    })
    await expect(link).toHaveAttribute('href', '/api/auth/sso/oidc/login')
    await expect(canvas.queryByLabelText("Nom d'utilisateur")).toBeNull()
  },
}

export const SsoLinkInvalid: Story = {
  beforeEach: () => {
    window.location.hash = 'token='
    return () => {
      history.replaceState(null, '', window.location.pathname + window.location.search)
    }
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(await canvas.findByText('Lien de connexion invalide ou expiré.')).toBeVisible()
  },
}

export const InvalidCredentials: Story = {
  decorators: [page(fakeAuthPort({ login: () => refused(401, 'invalid credentials') }))],
  play: async ({ canvasElement }) => {
    const canvas = await signIn(canvasElement)
    await expect(
      await canvas.findByText("Nom d'utilisateur ou mot de passe incorrect"),
    ).toBeVisible()
  },
}

export const MfaChallenge: Story = {
  decorators: [page(fakeAuthPort({ login: () => of(mfaPending()) }))],
  play: async ({ canvasElement }) => {
    const canvas = await signIn(canvasElement)
    await expect(
      await canvas.findByRole('heading', { name: 'Vérification en deux étapes' }),
    ).toBeVisible()
  },
}

export const MfaSetupRequired: Story = {
  decorators: [
    page(
      fakeAuthPort({
        login: () => of(mfaPending({ mfa_setup_required: true, mfa_has_totp: false })),
      }),
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = await signIn(canvasElement)
    await expect(
      await canvas.findByRole('heading', { name: 'Protégez votre compte' }),
    ).toBeVisible()
  },
}
