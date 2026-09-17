import {
  applicationConfig,
  moduleMetadata,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { provideRouter } from '@angular/router'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { of, throwError } from 'rxjs'
import { LoginPage } from './login-page'
import { AuthService } from '../application/auth.service'
import type { LoginOutcome, SsoConfig } from '../domain/auth.types'

/** A minimal stand-in for AuthService — only the methods each story actually exercises need a real implementation. */
function fakeAuth(overrides: Partial<AuthService> = {}): Partial<AuthService> {
  return {
    getSsoConfig: () => of<SsoConfig>({ type: null, registration_enabled: true }),
    login: () => of<LoginOutcome>({ mfaRequired: false }),
    ...overrides,
  }
}

/** Fills in and submits the local-login form — shared by every story that needs to get past it. */
async function submitLoginForm(
  canvasElement: HTMLElement,
  username = 'florian',
  password = 'hunter2',
) {
  const canvas = within(canvasElement)
  // Required-field labels get a trailing " *" appended by GbtInput — match by prefix.
  await userEvent.type(await canvas.findByLabelText(/^Nom d'utilisateur/), username)
  await userEvent.type(canvas.getByLabelText(/^Mot de passe/), password)
  await userEvent.click(canvas.getByRole('button', { name: 'Se connecter' }))
}

const meta: Meta<LoginPage> = {
  title: 'Auth/LoginPage',
  component: LoginPage,
  decorators: [
    applicationConfig({ providers: [provideRouter([])] }),
    moduleMetadata({ providers: [{ provide: AuthService, useValue: fakeAuth() }] }),
  ],
}
export default meta

type Story = StoryObj<LoginPage>

export const Default: Story = {}

export const LdapSso: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuthService,
          useValue: fakeAuth({
            getSsoConfig: () => of({ type: 'ldap', registration_enabled: true }),
          }),
        },
      ],
    }),
  ],
}

export const OidcSsoRegistrationDisabled: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuthService,
          useValue: fakeAuth({
            getSsoConfig: () => of({ type: 'oidc', registration_enabled: false }),
          }),
        },
      ],
    }),
  ],
}

export const InvalidCredentials: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuthService,
          useValue: fakeAuth({ login: () => throwError(() => new Error('unauthorized')) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await submitLoginForm(canvasElement)
    await waitFor(() =>
      expect(within(canvasElement).getByRole('alert')).toHaveTextContent('Identifiants invalides'),
    )
  },
}

/** A login that requires a second factor the account already has enrolled (TOTP + passkey). */
export const MfaRequired: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuthService,
          useValue: fakeAuth({
            login: () =>
              of<LoginOutcome>({
                mfaRequired: true,
                mfaToken: 'story-mfa-token',
                mfaSetupRequired: false,
                mfaHasTotp: true,
                mfaHasPasskey: true,
              }),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await submitLoginForm(canvasElement)
    await waitFor(() =>
      expect(within(canvasElement).getByLabelText(/^Code de vérification/)).toBeInTheDocument(),
    )
  },
}

/** A fresh account with no factor enrolled yet — routes straight into MfaEnrollmentPage. */
export const MfaSetupRequired: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuthService,
          useValue: fakeAuth({
            login: () =>
              of<LoginOutcome>({
                mfaRequired: true,
                mfaToken: 'story-mfa-token',
                mfaSetupRequired: true,
              }),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await submitLoginForm(canvasElement)
    await waitFor(() =>
      expect(
        within(canvasElement).getByText('Choisissez une méthode pour continuer :'),
      ).toBeInTheDocument(),
    )
  },
}
