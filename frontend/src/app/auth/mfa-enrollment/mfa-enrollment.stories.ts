import { Component, input } from '@angular/core'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { of, throwError } from 'rxjs'
import { MfaEnrollmentPage } from './mfa-enrollment'
import { AuthService } from '../application/auth.service'
import type { TotpSetupComplete, TotpSetupEnrollment } from '../domain/auth.types'

/**
 * MfaEnrollmentPage is a fragment — every real usage (LoginPage, RegisterPage) always embeds it
 * inside the shared auth-layout card, never renders it bare. This wrapper reproduces that exact
 * context, including the real stylesheet: Angular scopes component styles to the component that
 * declares them, so a plain HTML wrapper in a Storybook `render` template would not pick up
 * login-page.scss's `.auth-layout`/`.auth-layout__card` rules at all.
 */
@Component({
  selector: 'app-mfa-enrollment-story-wrapper',
  standalone: true,
  imports: [MfaEnrollmentPage],
  styleUrl: '../login-page/login-page.scss',
  template: `
    <main class="auth-layout">
      <div class="auth-layout__card">
        <img src="/api/branding/logo" alt="ArtiFerris logo" class="auth-layout__logo" />
        <app-mfa-enrollment [mfaToken]="mfaToken()" />
      </div>
    </main>
  `,
})
class MfaEnrollmentStoryWrapper {
  readonly mfaToken = input.required<string>()
}

function fakeAuth(overrides: Partial<AuthService> = {}): Partial<AuthService> {
  return {
    startTotpSetup: () =>
      of<TotpSetupEnrollment>({
        secret: 'JBSWY3DPEHPK3PXP',
        otpauth_url: 'otpauth://totp/ArtiFerris:florian?secret=JBSWY3DPEHPK3PXP&issuer=ArtiFerris',
      }),
    confirmTotpSetup: () =>
      of<TotpSetupComplete>({
        token: 'story-token',
        backup_codes: ['aaaa-1111', 'bbbb-2222', 'cccc-3333'],
      }),
    ...overrides,
  }
}

const meta: Meta<MfaEnrollmentStoryWrapper> = {
  title: 'Auth/MfaEnrollmentPage',
  component: MfaEnrollmentStoryWrapper,
  args: { mfaToken: 'story-mfa-token' },
  decorators: [moduleMetadata({ providers: [{ provide: AuthService, useValue: fakeAuth() }] })],
}
export default meta

type Story = StoryObj<MfaEnrollmentStoryWrapper>

/** The mandatory first-time enrollment screen — choice between TOTP and a passkey. */
export const Choice: Story = {}

export const TotpQrCode: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      await canvas.findByRole('button', { name: "Application d'authentification (TOTP)" }),
    )
    await waitFor(() => expect(canvas.getByText('JBSWY3DPEHPK3PXP')).toBeInTheDocument())
  },
}

export const BackupCodes: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      await canvas.findByRole('button', { name: "Application d'authentification (TOTP)" }),
    )
    await waitFor(() => canvas.getByLabelText('Code de vérification'))
    await userEvent.type(canvas.getByLabelText('Code de vérification'), '123456')
    await userEvent.click(canvas.getByRole('button', { name: 'Confirmer' }))
    await waitFor(() => expect(canvas.getByText('aaaa-1111')).toBeInTheDocument())
  },
}

export const InvalidTotpCode: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuthService,
          useValue: fakeAuth({ confirmTotpSetup: () => throwError(() => new Error('invalid')) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      await canvas.findByRole('button', { name: "Application d'authentification (TOTP)" }),
    )
    await waitFor(() => canvas.getByLabelText('Code de vérification'))
    await userEvent.type(canvas.getByLabelText('Code de vérification'), '000000')
    await userEvent.click(canvas.getByRole('button', { name: 'Confirmer' }))
    await waitFor(() => expect(canvas.getByRole('alert')).toHaveTextContent('Code invalide.'))
  },
}

/** Only reachable when the browser supports WebAuthn — see `passkeysSupported()`. */
export const PasskeySetup: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const passkeyButton = canvas.queryByRole('button', { name: "Clé d'accès (passkey)" })
    if (!passkeyButton) {
      // This story's browser (or CI's headless one) doesn't support WebAuthn — nothing to click.
      return
    }
    await userEvent.click(passkeyButton)
    await waitFor(() =>
      expect(canvas.getByLabelText('Nom de la clé (ex. « MacBook Touch ID »)')).toBeInTheDocument(),
    )
  },
}
