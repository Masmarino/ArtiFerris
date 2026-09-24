import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, spyOn, userEvent, waitFor, within } from 'storybook/test'
import { NEVER, of, throwError } from 'rxjs'
import { PasskeySettings } from './passkey-settings'
import { MfaService } from '../application/mfa.service'
import { ToastService } from '../../shared/toast.service'
import { SessionRevocationService } from '../../auth/application/session-revocation.service'
import type { PasskeySummary } from '../domain/mfa.types'

const MACBOOK: PasskeySummary = {
  id: 'pk1',
  name: 'MacBook Touch ID',
  created_at: '2026-06-01T00:00:00Z',
}
const YUBIKEY: PasskeySummary = {
  id: 'pk2',
  name: 'YubiKey 5C',
  created_at: '2026-07-01T00:00:00Z',
}

// base64url of bytes [1, 2, 3]
const CHALLENGE = 'AQID'

const REGISTRATION_START = {
  challenge_id: 'challenge-1',
  public_key: {
    challenge: CHALLENGE,
    rp: { name: 'ArtiFerris', id: 'localhost' },
    user: { id: CHALLENGE, name: 'florian', displayName: 'florian' },
    pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
  },
}

function fakeAuthenticatorCredential() {
  return {
    id: 'cred-1',
    type: 'public-key',
    rawId: new Uint8Array([1, 2, 3]).buffer,
    response: {
      attestationObject: new Uint8Array([4, 5]).buffer,
      clientDataJSON: new Uint8Array([6, 7]).buffer,
    },
  }
}

/** Replaces navigator.credentials for the story; returns the cleanup Storybook runs afterwards. */
function stubCredentials(value: Partial<CredentialsContainer> | undefined): () => void {
  Object.defineProperty(navigator, 'credentials', { configurable: true, value })
  return () => {
    delete (navigator as unknown as Record<string, unknown>)['credentials']
  }
}

const createCredential = fn(() => Promise.resolve(fakeAuthenticatorCredential()))

function fakeMfa(overrides: Partial<MfaService> = {}): Partial<MfaService> {
  return {
    listPasskeys: fn(() => of([MACBOOK])),
    startPasskeyRegistration: fn(() => of(REGISTRATION_START)),
    finishPasskeyRegistration: fn(() => of(undefined)),
    deletePasskey: fn(() => of(undefined)),
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

const meta: Meta<PasskeySettings> = {
  title: 'Account/PasskeySettings',
  component: PasskeySettings,
  decorators: [withServices(fakeMfa())],
  beforeEach: () => {
    createCredential.mockClear()
    return stubCredentials({
      create: createCredential as unknown as CredentialsContainer['create'],
    })
  },
}
export default meta

type Story = StoryObj<PasskeySettings>

export const Loading: Story = {
  decorators: [withServices(fakeMfa({ listPasskeys: () => NEVER }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('status')).toHaveTextContent('Chargement…')
    expect(
      canvas.queryByRole('button', { name: "Ajouter une clé d'accès" }),
    ).not.toBeInTheDocument()
  },
}

/** No WebAuthn in the browser: only the error, no list and no "Ajouter" button. */
export const BrowserWithoutPasskeySupport: Story = {
  beforeEach: () => stubCredentials(undefined),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('alert')).toHaveTextContent(
      "Votre navigateur ne prend pas en charge les clés d'accès.",
    )
    expect(
      canvas.queryByRole('button', { name: "Ajouter une clé d'accès" }),
    ).not.toBeInTheDocument()
  },
}

export const Empty: Story = {
  decorators: [withServices(fakeMfa({ listPasskeys: () => of([]) }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText("Aucune clé d'accès")).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: "Ajouter une clé d'accès" })).toBeEnabled()
  },
}

/** Each passkey has its own password field; its delete button stays disabled until it is filled. */
export const WithPasskeys: Story = {
  decorators: [withServices(fakeMfa({ listPasskeys: () => of([MACBOOK, YUBIKEY]) }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('MacBook Touch ID')).toBeInTheDocument()
    expect(canvas.getByText('YubiKey 5C')).toBeInTheDocument()
    const deleteButtons = canvas.getAllByRole('button', { name: 'Supprimer' })
    expect(deleteButtons).toHaveLength(2)
    deleteButtons.forEach((button) => expect(button).toBeDisabled())
    await userEvent.type(canvas.getAllByLabelText('Mot de passe actuel')[1], 'hunter2')
    await waitFor(() => expect(deleteButtons[1]).toBeEnabled())
    expect(deleteButtons[0]).toBeDisabled()
  },
}

export const LoadFailed: Story = {
  decorators: [
    withServices(fakeMfa({ listPasskeys: () => throwError(() => new Error('network error')) })),
  ],
  play: async ({ canvasElement }) => {
    expect(await within(canvasElement).findByRole('alert')).toHaveTextContent(
      "Échec du chargement des clés d'accès.",
    )
  },
}

export const CancellingTheAddForm: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: "Ajouter une clé d'accès" }))
    expect(await canvas.findByLabelText(/Nom de la clé/)).toBeInTheDocument()
    await userEvent.click(canvas.getByRole('button', { name: 'Annuler' }))
    expect(await canvas.findByRole('button', { name: "Ajouter une clé d'accès" })).toBeVisible()
    expect(canvas.queryByLabelText(/Nom de la clé/)).not.toBeInTheDocument()
  },
}

/** The register button stays disabled until the new key has a name. */
export const RegisterNeedsAName: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: "Ajouter une clé d'accès" }))
    const register = await canvas.findByRole('button', { name: 'Enregistrer une nouvelle clé' })
    expect(register).toBeDisabled()
    await userEvent.type(canvas.getByLabelText(/Nom de la clé/), 'YubiKey 5C')
    await waitFor(() => expect(register).toBeEnabled())
  },
}

let passkeysAfterRegistration: PasskeySummary[] = [MACBOOK]
const registerMfa = fakeMfa({ listPasskeys: fn(() => of(passkeysAfterRegistration)) })
const registerToasts = new ToastService()

/** The full ceremony runs against a faked authenticator; the list is reloaded afterwards. */
export const RegisteringAPasskey: Story = {
  decorators: [withServices(registerMfa, registerToasts)],
  beforeEach: () => {
    passkeysAfterRegistration = [MACBOOK]
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: "Ajouter une clé d'accès" }))
    await userEvent.type(await canvas.findByLabelText(/Nom de la clé/), 'YubiKey 5C')
    passkeysAfterRegistration = [MACBOOK, YUBIKEY]
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer une nouvelle clé' }))

    expect(await canvas.findByText('YubiKey 5C')).toBeInTheDocument()
    expect(createCredential).toHaveBeenCalledTimes(1)
    expect(registerMfa.finishPasskeyRegistration).toHaveBeenCalledWith(
      'challenge-1',
      {
        id: 'cred-1',
        rawId: 'AQID',
        type: 'public-key',
        response: { attestationObject: 'BAU', clientDataJSON: 'Bgc' },
      },
      'YubiKey 5C',
    )
    expect(registerToasts.toasts()).toEqual([
      expect.objectContaining({ variant: 'success', message: 'Clé d’accès enregistrée.' }),
    ])
    expect(canvas.queryByLabelText(/Nom de la clé/)).not.toBeInTheDocument()
  },
}

const cancelledCeremonyMfa = fakeMfa()
const cancelledCeremonyToasts = new ToastService()

/** The user dismisses the browser's passkey prompt: nothing is sent to the server, the form stays open. */
export const BrowserCeremonyFails: Story = {
  decorators: [withServices(cancelledCeremonyMfa, cancelledCeremonyToasts)],
  beforeEach: () => {
    const consoleError = spyOn(console, 'error').mockImplementation(() => undefined)
    const restoreCredentials = stubCredentials({
      create: () =>
        Promise.reject(new DOMException('The operation was cancelled.', 'NotAllowedError')),
    })
    return () => {
      restoreCredentials()
      consoleError.mockRestore()
    }
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: "Ajouter une clé d'accès" }))
    await userEvent.type(await canvas.findByLabelText(/Nom de la clé/), 'YubiKey 5C')
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer une nouvelle clé' }))

    await waitFor(() =>
      expect(cancelledCeremonyToasts.toasts()).toEqual([
        expect.objectContaining({
          variant: 'error',
          message: "Échec de l'enregistrement de la clé d'accès. Réessayez.",
        }),
      ]),
    )
    expect(cancelledCeremonyMfa.finishPasskeyRegistration).not.toHaveBeenCalled()
    expect(canvas.getByLabelText(/Nom de la clé/)).toHaveValue('YubiKey 5C')
    expect(canvas.getByRole('button', { name: 'Enregistrer une nouvelle clé' })).toBeEnabled()
  },
}

const serverRejectsMfa = fakeMfa({
  finishPasskeyRegistration: fn(() => throwError(() => new Error('attestation invalid'))),
})
const serverRejectsToasts = new ToastService()

export const ServerRejectsTheCredential: Story = {
  decorators: [withServices(serverRejectsMfa, serverRejectsToasts)],
  beforeEach: () => {
    const consoleError = spyOn(console, 'error').mockImplementation(() => undefined)
    return () => consoleError.mockRestore()
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: "Ajouter une clé d'accès" }))
    await userEvent.type(await canvas.findByLabelText(/Nom de la clé/), 'YubiKey 5C')
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer une nouvelle clé' }))

    await waitFor(() =>
      expect(serverRejectsToasts.toasts()).toEqual([
        expect.objectContaining({
          variant: 'error',
          message: "Échec de l'enregistrement de la clé d'accès. Réessayez.",
        }),
      ]),
    )
    expect(serverRejectsMfa.finishPasskeyRegistration).toHaveBeenCalled()
    expect(canvas.getByLabelText(/Nom de la clé/)).toBeInTheDocument()
  },
}

const deleteMfa = fakeMfa({ listPasskeys: fn(() => of([MACBOOK, YUBIKEY])) })
const deleteSignOut = fn()

/** Deleting a passkey ends the session on the backend: the user is signed out straight away. */
export const DeletingAPasskey: Story = {
  decorators: [withServices(deleteMfa, new ToastService(), deleteSignOut)],
  beforeEach: () => deleteSignOut.mockClear(),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('YubiKey 5C')).toBeInTheDocument()
    await userEvent.type(canvas.getAllByLabelText('Mot de passe actuel')[1], 'hunter2')
    await userEvent.click(canvas.getAllByRole('button', { name: 'Supprimer' })[1])

    await waitFor(() => expect(deleteSignOut).toHaveBeenCalledTimes(1))
    expect(deleteMfa.deletePasskey).toHaveBeenCalledWith('pk2', 'hunter2')
  },
}

const wrongPasswordToasts = new ToastService()

export const DeletingWithWrongPassword: Story = {
  decorators: [
    withServices(
      fakeMfa({ deletePasskey: () => throwError(() => new Error('400')) }),
      wrongPasswordToasts,
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await canvas.findByLabelText('Mot de passe actuel'), 'nope')
    await userEvent.click(canvas.getByRole('button', { name: 'Supprimer' }))

    await waitFor(() =>
      expect(wrongPasswordToasts.toasts()).toEqual([
        expect.objectContaining({ variant: 'error', message: 'Mot de passe incorrect.' }),
      ]),
    )
    expect(canvas.getByText('MacBook Touch ID')).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Supprimer' })).toBeEnabled()
  },
}
