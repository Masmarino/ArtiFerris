import { Component, input } from '@angular/core'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { of, throwError } from 'rxjs'
import { MfaEnrollmentPage } from './mfa-enrollment'
import { AuthService } from '../application/auth.service'
import type { TotpSetupComplete, TotpSetupEnrollment } from '../domain/auth.types'

// A fragment, always embedded in the shared auth-layout card: this wrapper reproduces that, with
// the real
// stylesheet (component styles are scoped, so a plain HTML wrapper would miss login-page.scss).
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

export const PasskeySetup: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const passkeyButton = canvas.queryByRole('button', { name: "Clé d'accès (passkey)" })
    if (!passkeyButton) {
      // This browser (or headless CI) has no WebAuthn: nothing to click.
      return
    }
    await userEvent.click(passkeyButton)
    await waitFor(() =>
      expect(canvas.getByLabelText('Nom de la clé (ex. « MacBook Touch ID »)')).toBeInTheDocument(),
    )
  },
}
