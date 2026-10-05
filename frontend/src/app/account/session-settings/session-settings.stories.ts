import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { of, throwError } from 'rxjs'
import { SessionSettings } from './session-settings'
import { AuthService } from '../../auth/application/auth.service'
import { SessionRevocationService } from '../../auth/application/session-revocation.service'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'

function withServices(
  logoutEverywhere: AuthService['logoutEverywhere'],
  confirmed: boolean,
  signOutAndRedirect = fn(),
  toasts = new ToastService(),
) {
  return moduleMetadata({
    providers: [
      { provide: AuthService, useValue: { logoutEverywhere } },
      { provide: ConfirmService, useValue: { ask: fn(() => Promise.resolve(confirmed)) } },
      { provide: SessionRevocationService, useValue: { signOutAndRedirect } },
      { provide: ToastService, useValue: toasts },
    ],
  })
}

const meta: Meta<SessionSettings> = {
  title: 'Account/SessionSettings',
  component: SessionSettings,
  decorators: [withServices(() => of(undefined), true)],
}
export default meta

type Story = StoryObj<SessionSettings>

export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('button', { name: 'Se déconnecter partout' })).toBeEnabled()
  },
}

const confirmedSignOut = fn()
export const SigningOutEverywhere: Story = {
  decorators: [withServices(() => of(undefined), true, confirmedSignOut)],
  beforeEach: () => confirmedSignOut.mockClear(),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Se déconnecter partout' }))
    await waitFor(() => expect(confirmedSignOut).toHaveBeenCalledTimes(1))
  },
}

const declinedSignOut = fn()
export const ConfirmationDeclined: Story = {
  decorators: [withServices(() => of(undefined), false, declinedSignOut)],
  beforeEach: () => declinedSignOut.mockClear(),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Se déconnecter partout' }))
    await new Promise((resolve) => setTimeout(resolve, 100))
    expect(declinedSignOut).not.toHaveBeenCalled()
  },
}

const failureToasts = new ToastService()
export const ServerFailure: Story = {
  decorators: [withServices(() => throwError(() => new Error('boom')), true, fn(), failureToasts)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Se déconnecter partout' }))
    await waitFor(() =>
      expect(failureToasts.toasts().at(-1)?.message).toBe(
        'Impossible de fermer les sessions. Réessayez.',
      ),
    )
  },
}
