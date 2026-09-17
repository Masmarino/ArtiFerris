import {
  applicationConfig,
  moduleMetadata,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { provideRouter } from '@angular/router'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { of, throwError } from 'rxjs'
import { RegisterPage } from './register-page'
import { AuthService } from '../application/auth.service'
import type { LoginOutcome } from '../domain/auth.types'

function fakeAuth(overrides: Partial<AuthService> = {}): Partial<AuthService> {
  return {
    register: () =>
      of<LoginOutcome>({ mfaRequired: true, mfaToken: 'story-mfa-token', mfaSetupRequired: true }),
    ...overrides,
  }
}

async function fillRegisterForm(canvasElement: HTMLElement) {
  const canvas = within(canvasElement)
  // Required-field labels get a trailing " *" appended by GbtInput — match by prefix.
  await userEvent.type(await canvas.findByLabelText(/^Nom d'utilisateur/), 'florian')
  await userEvent.type(canvas.getByLabelText(/^Adresse e-mail/), 'florian@example.com')
  await userEvent.type(canvas.getByLabelText(/^Mot de passe/), 'hunter2222')
}

const meta: Meta<RegisterPage> = {
  title: 'Auth/RegisterPage',
  component: RegisterPage,
  decorators: [
    applicationConfig({ providers: [provideRouter([])] }),
    moduleMetadata({ providers: [{ provide: AuthService, useValue: fakeAuth() }] }),
  ],
}
export default meta

type Story = StoryObj<RegisterPage>

/** The registration form, before it's ever been submitted. */
export const Default: Story = {}

/** After a successful registration, the account still needs its first MFA factor. */
export const AfterSubmitEntersMfaSetup: Story = {
  play: async ({ canvasElement }) => {
    await fillRegisterForm(canvasElement)
    await userEvent.click(within(canvasElement).getByRole('button', { name: 'Créer mon compte' }))
    await waitFor(() =>
      expect(
        within(canvasElement).getByText('Choisissez une méthode pour continuer :'),
      ).toBeInTheDocument(),
    )
  },
}

export const UsernameAlreadyTaken: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuthService,
          useValue: fakeAuth({
            register: () => throwError(() => ({ error: { error: 'username already taken' } })),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await fillRegisterForm(canvasElement)
    await userEvent.click(within(canvasElement).getByRole('button', { name: 'Créer mon compte' }))
    await waitFor(() =>
      expect(within(canvasElement).getByRole('alert')).toHaveTextContent(
        'Ce nom d’utilisateur est déjà pris.',
      ),
    )
  },
}

export const RegistrationDisabled: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuthService,
          useValue: fakeAuth({
            register: () => throwError(() => ({ error: { error: 'currently disabled' } })),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await fillRegisterForm(canvasElement)
    await userEvent.click(within(canvasElement).getByRole('button', { name: 'Créer mon compte' }))
    await waitFor(() =>
      expect(within(canvasElement).getByRole('alert')).toHaveTextContent(
        "La création de compte est actuellement désactivée par l'administrateur.",
      ),
    )
  },
}
