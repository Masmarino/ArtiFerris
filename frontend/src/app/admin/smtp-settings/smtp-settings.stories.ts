import { HttpErrorResponse } from '@angular/common/http'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { NEVER, of, throwError } from 'rxjs'
import { SmtpSettingsAdmin } from './smtp-settings'
import { SmtpSettingsService } from '../application/smtp-settings.service'
import { ToastService } from '../../shared/toast.service'
import type { SmtpSettings } from '../domain/smtp-settings.entity'

const CONFIGURED: SmtpSettings = {
  host: 'smtp.example.com',
  port: 587,
  username: 'artiferris@example.com',
  from_name: 'Acme Corp',
  from_address: 'artiferris@example.com',
  security: 'start_tls',
  password_set: true,
}

function fakeSmtp(overrides: Partial<SmtpSettingsService> = {}): Partial<SmtpSettingsService> {
  return {
    get: fn(() => of<SmtpSettings | null>(null)),
    update: fn(() => of(undefined)),
    sendTestEmail: fn(() => of(undefined)),
    ...overrides,
  }
}

const toast = { success: fn(), error: fn() }

function withSmtp(smtp: Partial<SmtpSettingsService>) {
  return moduleMetadata({ providers: [{ provide: SmtpSettingsService, useValue: smtp }] })
}

async function waitForForm(canvas: ReturnType<typeof within>) {
  await waitFor(() => expect(canvas.getByLabelText('Hôte SMTP')).toBeInTheDocument())
}

async function fillRequiredFields(canvas: ReturnType<typeof within>) {
  await userEvent.type(canvas.getByLabelText('Hôte SMTP'), 'smtp.acme.test')
  await userEvent.type(canvas.getByLabelText('Identifiant'), 'mailer')
  await userEvent.type(canvas.getByLabelText('Mot de passe'), 's3cret')
  await userEvent.type(canvas.getByLabelText("Adresse d'expédition"), 'noreply@acme.test')
}

const meta: Meta<SmtpSettingsAdmin> = {
  title: 'Admin/SmtpSettingsAdmin',
  component: SmtpSettingsAdmin,
  beforeEach: () => {
    toast.success.mockClear()
    toast.error.mockClear()
  },
  decorators: [
    moduleMetadata({
      providers: [
        { provide: SmtpSettingsService, useValue: fakeSmtp() },
        { provide: ToastService, useValue: toast },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<SmtpSettingsAdmin>

/** SMTP has never been configured: defaults only, no errors shown before a save is attempted. */
export const NotConfigured: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    expect(canvas.getByLabelText('Hôte SMTP')).toHaveValue('')
    expect(canvas.getByLabelText('Port')).toHaveValue('587')
    expect(canvas.getByLabelText("Nom d'expéditeur")).toHaveValue('ArtiFerris')
    expect(canvas.getByRole('button', { name: 'Enregistrer' })).toBeEnabled()
    expect(canvas.queryByText("L'hôte est requis.")).not.toBeInTheDocument()
  },
}

const loadingSmtp = fakeSmtp({ get: () => NEVER })
export const Loading: Story = {
  decorators: [withSmtp(loadingSmtp)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('Chargement…')).toBeInTheDocument())
    expect(canvas.queryByLabelText('Hôte SMTP')).not.toBeInTheDocument()
    expect(canvas.queryByRole('button', { name: 'Envoyer' })).not.toBeInTheDocument()
  },
}

/** Stored settings are loaded; the password itself is never sent back, only that one is set. */
export const Configured: Story = {
  decorators: [withSmtp(fakeSmtp({ get: () => of(CONFIGURED) }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByLabelText('Hôte SMTP')).toHaveValue('smtp.example.com'))
    expect(canvas.getByLabelText('Identifiant')).toHaveValue('artiferris@example.com')
    expect(canvas.getByLabelText('Mot de passe')).toHaveValue('')
    expect(
      canvas.getByPlaceholderText('Laisser vide pour conserver le mot de passe actuel'),
    ).toBeInTheDocument()
    expect(canvas.getByLabelText("Nom d'expéditeur")).toHaveValue('Acme Corp')
  },
}

const scopedSmtp = fakeSmtp({ get: fn(() => of(CONFIGURED)) })
/** Embedded in one organization's admin page: read and write are scoped to it. */
export const ScopedToAnOrganization: Story = {
  args: { organizationId: 'org-acme' },
  decorators: [withSmtp(scopedSmtp)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByLabelText('Hôte SMTP')).toHaveValue('smtp.example.com'))
    expect(scopedSmtp.get).toHaveBeenCalledWith('org-acme')

    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))
    await waitFor(() =>
      expect(scopedSmtp.update).toHaveBeenCalledWith(expect.anything(), 'org-acme'),
    )
  },
}

/** Saving an empty form reveals every field error and never calls the API. */
export const ValidationErrors: Story = {
  decorators: [withSmtp(fakeSmtp())],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    expect(await canvas.findByText("L'hôte est requis.")).toBeInTheDocument()
    expect(canvas.getByText("L'identifiant est requis.")).toBeInTheDocument()
    expect(canvas.getByText('Doit être une adresse e-mail valide.')).toBeInTheDocument()
    expect(
      canvas.getByText('Le mot de passe est requis lors de la première configuration.'),
    ).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Enregistrer' })).toBeDisabled()
  },
}

const invalidPortSmtp = fakeSmtp()
export const InvalidPort: Story = {
  decorators: [withSmtp(invalidPortSmtp)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    await fillRequiredFields(canvas)
    const port = canvas.getByLabelText('Port')
    await userEvent.clear(port)
    await userEvent.type(port, '70000')
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    expect(await canvas.findByText('Doit être entre 1 et 65535.')).toBeInTheDocument()
    expect(invalidPortSmtp.update).not.toHaveBeenCalled()

    await userEvent.clear(port)
    await userEvent.type(port, 'abc')
    expect(await canvas.findByText('Doit être un nombre entier.')).toBeInTheDocument()
  },
}

const invalidEmailSmtp = fakeSmtp()
export const InvalidSenderAddress: Story = {
  decorators: [withSmtp(invalidEmailSmtp)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    await fillRequiredFields(canvas)
    const from = canvas.getByLabelText("Adresse d'expédition")
    await userEvent.clear(from)
    await userEvent.type(from, 'not-an-email')
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    expect(await canvas.findByText('Doit être une adresse e-mail valide.')).toBeInTheDocument()
    expect(invalidEmailSmtp.update).not.toHaveBeenCalled()
  },
}

const savingSmtp = fakeSmtp({ update: fn(() => NEVER) })
/** The save request is in flight: the button shows its busy state and can't be clicked again. */
export const Saving: Story = {
  decorators: [withSmtp(savingSmtp)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    await fillRequiredFields(canvas)
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    const button = await canvas.findByRole('button', { name: /Enregistrer/ })
    await waitFor(() => expect(button).toHaveAttribute('aria-busy', 'true'))
    expect(button).toBeDisabled()
    expect(savingSmtp.update).toHaveBeenCalledTimes(1)
  },
}

const savedSmtp = fakeSmtp()
export const SavedSuccessfully: Story = {
  decorators: [withSmtp(savedSmtp)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    await fillRequiredFields(canvas)
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(savedSmtp.update).toHaveBeenCalledWith(
        {
          host: 'smtp.acme.test',
          port: 587,
          username: 'mailer',
          password: 's3cret',
          from_name: 'ArtiFerris',
          from_address: 'noreply@acme.test',
          security: 'start_tls',
        },
        undefined,
      ),
    )
    expect(toast.success).toHaveBeenCalledWith('Paramètres SMTP enregistrés.')
    // The typed password is cleared and the placeholder now says one is stored.
    expect(canvas.getByLabelText('Mot de passe')).toHaveValue('')
    expect(
      canvas.getByPlaceholderText('Laisser vide pour conserver le mot de passe actuel'),
    ).toBeInTheDocument()
  },
}

const keepPasswordSmtp = fakeSmtp({ get: () => of(CONFIGURED) })
/** With a password already stored, leaving the field blank omits it from the update. */
export const SavingWithoutChangingThePassword: Story = {
  decorators: [withSmtp(keepPasswordSmtp)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByLabelText('Hôte SMTP')).toHaveValue('smtp.example.com'))
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(keepPasswordSmtp.update).toHaveBeenCalledWith(
        expect.objectContaining({ host: 'smtp.example.com', password: undefined }),
        undefined,
      ),
    )
  },
}

const changeSecuritySmtp = fakeSmtp({ get: () => of(CONFIGURED) })
export const ChangingTheSecurityMode: Story = {
  decorators: [withSmtp(changeSecuritySmtp)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByLabelText('Hôte SMTP')).toHaveValue('smtp.example.com'))
    await userEvent.click(canvas.getByRole('combobox', { name: 'Sécurité' }))
    await userEvent.click(
      await canvas.findByRole('option', { name: 'TLS implicite (port 465 usuellement)' }),
    )
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(changeSecuritySmtp.update).toHaveBeenCalledWith(
        expect.objectContaining({ security: 'tls' }),
        undefined,
      ),
    )
  },
}

const passwordUnreadableSmtp = fakeSmtp({
  get: () =>
    of({
      secret_unreadable: true as const,
      error: "the stored SMTP password cannot be read with this server's SECRETS_ENCRYPTION_KEY",
    }),
})

/** The stored password cannot be decrypted by this server: a warning is shown and the password has to be typed again. */
export const PasswordUnreadable: Story = {
  decorators: [withSmtp(passwordUnreadableSmtp)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    expect(canvas.getByRole('alert')).toHaveTextContent(
      'Le mot de passe SMTP enregistré est illisible',
    )
    expect(
      canvas.queryByPlaceholderText('Laisser vide pour conserver le mot de passe actuel'),
    ).toBe(null)
  },
}

const keptPasswordRefusedSmtp = fakeSmtp({
  get: () => of(CONFIGURED),
  update: fn(() =>
    throwError(
      () =>
        new HttpErrorResponse({
          status: 400,
          error: { error: 're-enter the password when changing the host, port or username' },
        }),
    ),
  ),
})

/** Changing the host without retyping the password is refused: the server's own message is shown. */
export const KeptPasswordRefusedAfterChangingTheHost: Story = {
  decorators: [withSmtp(keptPasswordRefusedSmtp)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByLabelText('Hôte SMTP')).toHaveValue('smtp.example.com'))
    await userEvent.clear(canvas.getByLabelText('Hôte SMTP'))
    await userEvent.type(canvas.getByLabelText('Hôte SMTP'), 'other.example.net')
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith(
        're-enter the password when changing the host, port or username',
      ),
    )
    expect(toast.success).not.toHaveBeenCalled()
  },
}

const failingSaveSmtp = fakeSmtp({ update: fn(() => throwError(() => new Error('boom'))) })
export const SaveFailed: Story = {
  decorators: [withSmtp(failingSaveSmtp)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    await fillRequiredFields(canvas)
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith('Échec de la mise à jour des paramètres SMTP.'),
    )
    expect(toast.success).not.toHaveBeenCalled()
    // The typed values are kept so the user can retry.
    expect(canvas.getByLabelText('Hôte SMTP')).toHaveValue('smtp.acme.test')
    expect(canvas.getByRole('button', { name: 'Enregistrer' })).toBeEnabled()
  },
}

/** "Envoyer" stays disabled until a recipient is typed. */
export const TestEmailNeedsARecipient: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByLabelText('Destinataire')).toBeInTheDocument())
    expect(canvas.getByRole('button', { name: 'Envoyer' })).toBeDisabled()

    await userEvent.type(canvas.getByLabelText('Destinataire'), 'me@acme.test')
    await waitFor(() => expect(canvas.getByRole('button', { name: 'Envoyer' })).toBeEnabled())
  },
}

const testEmailSmtp = fakeSmtp({ get: () => of(CONFIGURED) })
export const SendingATestEmail: Story = {
  args: { organizationId: 'org-acme' },
  decorators: [withSmtp(testEmailSmtp)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await canvas.findByLabelText('Destinataire'), 'me@acme.test')
    await userEvent.click(canvas.getByRole('button', { name: 'Envoyer' }))

    await waitFor(() =>
      expect(testEmailSmtp.sendTestEmail).toHaveBeenCalledWith('me@acme.test', 'org-acme'),
    )
    expect(toast.success).toHaveBeenCalledWith('E-mail de test envoyé.')
    expect(testEmailSmtp.update).not.toHaveBeenCalled()
  },
}

const sendingTestSmtp = fakeSmtp({ sendTestEmail: fn(() => NEVER) })
export const TestEmailInFlight: Story = {
  decorators: [withSmtp(sendingTestSmtp)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await canvas.findByLabelText('Destinataire'), 'me@acme.test')
    await userEvent.click(canvas.getByRole('button', { name: 'Envoyer' }))

    const button = await canvas.findByRole('button', { name: /Envoyer/ })
    await waitFor(() => expect(button).toHaveAttribute('aria-busy', 'true'))
    expect(button).toBeDisabled()
  },
}

const failingTestSmtp = fakeSmtp({ sendTestEmail: fn(() => throwError(() => new Error('boom'))) })
export const TestEmailFailed: Story = {
  decorators: [withSmtp(failingTestSmtp)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await canvas.findByLabelText('Destinataire'), 'me@acme.test')
    await userEvent.click(canvas.getByRole('button', { name: 'Envoyer' }))

    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith(
        "Échec de l'envoi de l'e-mail de test. Vérifiez la configuration.",
      ),
    )
    expect(toast.success).not.toHaveBeenCalled()
    expect(canvas.getByRole('button', { name: 'Envoyer' })).toBeEnabled()
  },
}
