import {
  applicationConfig,
  moduleMetadata,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { of, throwError } from 'rxjs'
import { ActivatePage } from './activate-page'
import { AuthService } from '../application/auth.service'

function fakeAuth(overrides: Partial<AuthService> = {}): Partial<AuthService> {
  return {
    activate: () => of(undefined),
    ...overrides,
  }
}

function withToken(token: string | null) {
  return {
    provide: ActivatedRoute,
    useValue: { snapshot: { queryParamMap: convertToParamMap(token === null ? {} : { token }) } },
  }
}

const meta: Meta<ActivatePage> = {
  title: 'Auth/ActivatePage',
  component: ActivatePage,
  decorators: [
    applicationConfig({ providers: [provideRouter([])] }),
    moduleMetadata({
      providers: [{ provide: AuthService, useValue: fakeAuth() }, withToken('a-valid-token')],
    }),
  ],
}
export default meta

type Story = StoryObj<ActivatePage>

/** A valid activation link — the normal, expected entry point. */
export const Default: Story = {}

/** Someone opened the page directly, with no `?token=` in the URL. */
export const MissingToken: Story = {
  decorators: [moduleMetadata({ providers: [withToken(null)] })],
}

export const ExpiredOrInvalidToken: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuthService,
          useValue: fakeAuth({ activate: () => throwError(() => new Error('gone')) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // Required-field labels get a trailing " *" appended by GbtInput — match by prefix.
    await userEvent.type(await canvas.findByLabelText(/^Nouveau mot de passe/), 'hunter2222')
    await userEvent.type(canvas.getByLabelText(/^Confirmer le mot de passe/), 'hunter2222')
    await userEvent.click(canvas.getByRole('button', { name: 'Activer mon compte' }))
    await waitFor(() =>
      expect(canvas.getByRole('alert')).toHaveTextContent(
        "Ce lien d'activation est invalide ou a expiré",
      ),
    )
  },
}

/** The client-side mismatch check, before the form is even submitted. */
export const PasswordMismatch: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await canvas.findByLabelText(/^Nouveau mot de passe/), 'hunter2222')
    await userEvent.type(canvas.getByLabelText(/^Confirmer le mot de passe/), 'somethingElse')
    await userEvent.tab()
    await waitFor(() =>
      expect(canvas.getByText('Les mots de passe ne correspondent pas.')).toBeInTheDocument(),
    )
  },
}
