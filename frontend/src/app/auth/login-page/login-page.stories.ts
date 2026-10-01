import { HttpErrorResponse } from '@angular/common/http'
import {
  applicationConfig,
  moduleMetadata,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { of, throwError } from 'rxjs'
import { LoginPage } from './login-page'
import { AuthService } from '../application/auth.service'
import type { LoginOutcome, SsoConfig } from '../domain/auth.types'

function fakeAuth(overrides: Partial<AuthService> = {}): Partial<AuthService> {
  return {
    getSsoConfig: () => of<SsoConfig>({ type: null, registration_enabled: true }),
    login: () => of<LoginOutcome>({ mfaRequired: false }),
    abandonSsoLogin: () => undefined,
    ...overrides,
  }
}

async function submitLoginForm(
  canvasElement: HTMLElement,
  username = 'florian',
  password = 'hunter2',
) {
  const canvas = within(canvasElement)
  // GbtInput appends " *" to required-field labels: match by prefix.
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

export const Default: Story = {
  play: async ({ canvasElement }) => {
    const link = await within(canvasElement).findByRole('link', {
      name: 'Explorer les paquets publics',
    })
    expect(link).toHaveAttribute('href', expect.stringMatching(/\/explorer$/))
  },
}

export const AfterSessionsWereEnded: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { queryParamMap: convertToParamMap({ reason: 'sessions-revoked' }) },
          },
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(
      await canvas.findByText('Vos sessions ont été fermées : reconnectez-vous.'),
    ).toBeVisible()
    expect(canvas.getByRole('button', { name: 'Se connecter' })).toBeInTheDocument()
  },
}

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

export const ServerBusy: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuthService,
          useValue: fakeAuth({
            login: () => throwError(() => new HttpErrorResponse({ status: 503 })),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await submitLoginForm(canvasElement)
    await waitFor(() =>
      expect(within(canvasElement).getByRole('alert')).toHaveTextContent(
        'Service momentanément occupé, réessayez',
      ),
    )
  },
}

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
