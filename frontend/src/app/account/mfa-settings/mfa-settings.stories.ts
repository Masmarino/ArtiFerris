import { HttpErrorResponse } from '@angular/common/http'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { NEVER, of, throwError } from 'rxjs'
import { MfaSettings } from './mfa-settings'
import { MfaService } from '../application/mfa.service'
import { ToastService } from '../../shared/toast.service'
import { SessionRevocationService } from '../../auth/application/session-revocation.service'
import type { MfaStatus } from '../domain/mfa.types'

const DISABLED: MfaStatus = { totp_enabled: false, backup_codes_remaining: 0, passkey_count: 0 }
const ENABLED: MfaStatus = { totp_enabled: true, backup_codes_remaining: 6, passkey_count: 0 }

const BACKUP_CODES = ['aaaa-1111', 'bbbb-2222', 'cccc-3333']

function fakeMfa(overrides: Partial<MfaService> = {}): Partial<MfaService> {
  return {
    getStatus: fn(() => of(DISABLED)),
    enrollTotp: fn(() =>
      of({ secret: 'ABCD EFGH IJKL', otpauth_url: 'otpauth://totp/ArtiFerris:florian' }),
    ),
    confirmTotp: fn(() => of({ backup_codes: BACKUP_CODES })),
    disableTotp: fn(() => of(undefined)),
    regenerateBackupCodes: fn(() => of({ backup_codes: BACKUP_CODES })),
    ...overrides,
  }
}

function withServices(
  mfa: Partial<MfaService>,
  toasts = new ToastService(),
  signOutAndRedirect = fn(),
) {
  return moduleMetadata({
    providers: [
      { provide: MfaService, useValue: mfa },
      { provide: ToastService, useValue: toasts },
      { provide: SessionRevocationService, useValue: { signOutAndRedirect } },
    ],
  })
}

function wrongPassword(): HttpErrorResponse {
  return new HttpErrorResponse({ status: 400 })
}

const meta: Meta<MfaSettings> = {
  title: 'Account/MfaSettings',
  component: MfaSettings,
  decorators: [withServices(fakeMfa())],
}
export default meta

type Story = StoryObj<MfaSettings>

export const Loading: Story = {
  decorators: [withServices(fakeMfa({ getStatus: () => NEVER }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Chargement…')).toBeInTheDocument()
    expect(canvas.queryByRole('button', { name: 'Activer' })).not.toBeInTheDocument()
  },
}

export const LoadFailed: Story = {
  decorators: [withServices(fakeMfa({ getStatus: () => throwError(() => new Error('boom')) }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('alert')).toHaveTextContent(
      'Échec du chargement de la double authentification.',
    )
    expect(canvas.queryByText('Chargement…')).not.toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Réessayer' })).toBeEnabled()
  },
}

export const Disabled: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(
      await canvas.findByText("La double authentification n'est pas activée sur ce compte."),
    ).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Activer' })).toBeEnabled()
  },
}

export const EnrollingInTotp: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Activer' }))
    expect(await canvas.findByText('ABCD EFGH IJKL')).toBeInTheDocument()
    expect(await canvas.findByRole('img', { name: "QR code d'activation" })).toBeInTheDocument()
    const confirm = canvas.getByRole('button', { name: 'Confirmer' })
    expect(confirm).toBeDisabled()
    await userEvent.type(canvas.getByLabelText('Code de vérification'), '123456')
    await waitFor(() => expect(confirm).toBeEnabled())
  },
}

export const EnrollmentCouldNotStart: Story = {
  decorators: [withServices(fakeMfa({ enrollTotp: () => throwError(() => new Error('boom')) }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Activer' }))
    expect(await canvas.findByRole('alert')).toHaveTextContent(
      "Échec du démarrage de l'activation. Réessayez.",
    )
    expect(canvas.getByRole('button', { name: 'Activer' })).toBeEnabled()
  },
}

export const CancellingEnrollment: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Activer' }))
    await userEvent.click(await canvas.findByRole('button', { name: 'Annuler' }))
    expect(await canvas.findByRole('button', { name: 'Activer' })).toBeInTheDocument()
    expect(canvas.queryByText('ABCD EFGH IJKL')).not.toBeInTheDocument()
  },
}

const wrongCodeMfa = fakeMfa({ confirmTotp: fn(() => throwError(() => wrongPassword())) })

export const EnrollmentWithInvalidCode: Story = {
  decorators: [withServices(wrongCodeMfa)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Activer' }))
    await userEvent.type(await canvas.findByLabelText('Code de vérification'), '000000')
    await userEvent.click(canvas.getByRole('button', { name: 'Confirmer' }))
    await waitFor(() => expect(canvas.getByRole('alert')).toHaveTextContent('Code invalide.'))
    expect(wrongCodeMfa.confirmTotp).toHaveBeenCalledWith('000000')
    expect(canvas.getByRole('button', { name: 'Confirmer' })).toBeEnabled()
  },
}

let statusAfterEnrollment: MfaStatus = DISABLED
const enrollmentMfa = fakeMfa({ getStatus: fn(() => of(statusAfterEnrollment)) })

export const EnrollmentSucceedsWithBackupCodes: Story = {
  decorators: [withServices(enrollmentMfa)],
  beforeEach: () => {
    statusAfterEnrollment = DISABLED
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Activer' }))
    await userEvent.type(await canvas.findByLabelText('Code de vérification'), '123456')
    await userEvent.click(canvas.getByRole('button', { name: 'Confirmer' }))
    expect(await canvas.findByText('aaaa-1111')).toBeInTheDocument()
    expect(canvas.getByText('cccc-3333')).toBeInTheDocument()
    expect(enrollmentMfa.confirmTotp).toHaveBeenCalledWith('123456')

    statusAfterEnrollment = ENABLED
    await userEvent.click(canvas.getByRole('button', { name: "J'ai sauvegardé ces codes" }))
    expect(await canvas.findByText('Codes de secours restants : 6')).toBeInTheDocument()
    expect(canvas.queryByText('aaaa-1111')).not.toBeInTheDocument()
  },
}

export const Enabled: Story = {
  decorators: [withServices(fakeMfa({ getStatus: () => of(ENABLED) }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Codes de secours restants : 6')).toBeInTheDocument()
    expect(canvas.getAllByLabelText('Mot de passe actuel')).toHaveLength(2)
    expect(
      canvas.getByRole('button', { name: 'Régénérer les codes de secours' }),
    ).toBeInTheDocument()
    expect(
      canvas.getByRole('button', { name: 'Désactiver la double authentification' }),
    ).toBeInTheDocument()
  },
}

const regenMfa = fakeMfa({ getStatus: fn(() => of(ENABLED)) })
const regenSignOut = fn()

export const RegeneratingBackupCodes: Story = {
  decorators: [withServices(regenMfa, new ToastService(), regenSignOut)],
  beforeEach: () => regenSignOut.mockClear(),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const [regeneratePassword] = await canvas.findAllByLabelText('Mot de passe actuel')
    await userEvent.type(regeneratePassword, 'hunter2')
    await userEvent.click(canvas.getByRole('button', { name: 'Régénérer les codes de secours' }))
    expect(await canvas.findByText('bbbb-2222')).toBeInTheDocument()
    expect(regenMfa.regenerateBackupCodes).toHaveBeenCalledWith('hunter2')
    expect(canvas.getByRole('status')).toHaveTextContent('Vos sessions vont être fermées')
    expect(regenSignOut).not.toHaveBeenCalled()

    await userEvent.click(
      canvas.getByRole('button', { name: "J'ai sauvegardé ces codes, me reconnecter" }),
    )
    expect(regenSignOut).toHaveBeenCalledTimes(1)
  },
}

const regenWrongToasts = new ToastService()

export const RegenerationWithWrongPassword: Story = {
  decorators: [
    withServices(
      fakeMfa({
        getStatus: () => of(ENABLED),
        regenerateBackupCodes: () => throwError(() => wrongPassword()),
      }),
      regenWrongToasts,
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const [regeneratePassword] = await canvas.findAllByLabelText('Mot de passe actuel')
    await userEvent.type(regeneratePassword, 'nope')
    await userEvent.click(canvas.getByRole('button', { name: 'Régénérer les codes de secours' }))
    await waitFor(() =>
      expect(regenWrongToasts.toasts()).toEqual([
        expect.objectContaining({ variant: 'error', message: 'Mot de passe incorrect.' }),
      ]),
    )
    expect(canvas.queryByText('bbbb-2222')).not.toBeInTheDocument()
  },
}

const regenServerErrorToasts = new ToastService()

export const RegenerationServerError: Story = {
  decorators: [
    withServices(
      fakeMfa({
        getStatus: () => of(ENABLED),
        regenerateBackupCodes: () => throwError(() => new HttpErrorResponse({ status: 500 })),
      }),
      regenServerErrorToasts,
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const [regeneratePassword] = await canvas.findAllByLabelText('Mot de passe actuel')
    await userEvent.type(regeneratePassword, 'hunter2')
    await userEvent.click(canvas.getByRole('button', { name: 'Régénérer les codes de secours' }))
    await waitFor(() =>
      expect(regenServerErrorToasts.toasts()).toEqual([
        expect.objectContaining({ variant: 'error', message: 'Échec de la régénération.' }),
      ]),
    )
  },
}

const disableMfa = fakeMfa({ getStatus: fn(() => of(ENABLED)) })
const disableSignOut = fn()

export const DisablingMfa: Story = {
  decorators: [withServices(disableMfa, new ToastService(), disableSignOut)],
  beforeEach: () => disableSignOut.mockClear(),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const passwords = await canvas.findAllByLabelText('Mot de passe actuel')
    await userEvent.type(passwords[1], 'hunter2')
    await userEvent.click(
      canvas.getByRole('button', { name: 'Désactiver la double authentification' }),
    )
    await waitFor(() => expect(disableSignOut).toHaveBeenCalledTimes(1))
    expect(disableMfa.disableTotp).toHaveBeenCalledWith('hunter2')
  },
}

const disableWrongMfa = fakeMfa({
  getStatus: () => of(ENABLED),
  disableTotp: fn(() => throwError(() => wrongPassword())),
})
const disableWrongToasts = new ToastService()

export const DisablingWithWrongPassword: Story = {
  decorators: [withServices(disableWrongMfa, disableWrongToasts)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const passwords = await canvas.findAllByLabelText('Mot de passe actuel')
    await userEvent.type(passwords[1], 'nope')
    await userEvent.click(
      canvas.getByRole('button', { name: 'Désactiver la double authentification' }),
    )
    await waitFor(() =>
      expect(disableWrongToasts.toasts()).toEqual([
        expect.objectContaining({ variant: 'error', message: 'Mot de passe incorrect.' }),
      ]),
    )
    expect(canvas.getByText('Codes de secours restants : 6')).toBeInTheDocument()
  },
}

const emptyPasswordMfa = fakeMfa({ getStatus: () => of(ENABLED) })

export const EmptyPasswordIsIgnored: Story = {
  decorators: [withServices(emptyPasswordMfa)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      await canvas.findByRole('button', { name: 'Désactiver la double authentification' }),
    )
    await userEvent.click(canvas.getByRole('button', { name: 'Régénérer les codes de secours' }))
    expect(emptyPasswordMfa.disableTotp).not.toHaveBeenCalled()
    expect(emptyPasswordMfa.regenerateBackupCodes).not.toHaveBeenCalled()
  },
}
