import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { signal } from '@angular/core'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { of } from 'rxjs'
import { AccountPage } from './account-page'
import { MeService } from '../../shell/application/me.service'
import { MfaService } from '../application/mfa.service'
import { ApiTokensApplicationService } from '../../tokens/application/api-tokens.application-service'
import type { MfaStatus, PasskeySummary } from '../domain/mfa.types'
import type { ApiToken } from '../../tokens/domain/api-token.entity'

const DISABLED_STATUS: MfaStatus = {
  totp_enabled: false,
  backup_codes_remaining: 0,
  passkey_count: 0,
}

const ENABLED_STATUS: MfaStatus = {
  totp_enabled: true,
  backup_codes_remaining: 6,
  passkey_count: 1,
}

const PASSKEY: PasskeySummary = {
  id: 'pk1',
  name: 'MacBook Touch ID',
  created_at: '2026-06-01T00:00:00Z',
}

const TOKEN: ApiToken = {
  id: 't1',
  label: 'mon laptop',
  created_at: '2026-06-01T00:00:00Z',
  last_used_at: null,
}

function fakeMe(isSuperAdmin = false): Partial<MeService> {
  return {
    username: signal('florian'),
    isSuperAdmin: signal(isSuperAdmin),
    createdAt: signal('2026-01-01T00:00:00Z'),
    changePassword: () => of(undefined),
  }
}

function fakeMfa(overrides: Partial<MfaService> = {}): Partial<MfaService> {
  return {
    getStatus: () => of(DISABLED_STATUS),
    listPasskeys: () => of([]),
    ...overrides,
  }
}

function fakeTokens(
  overrides: Partial<ApiTokensApplicationService> = {},
): Partial<ApiTokensApplicationService> {
  return { list: () => of([]), ...overrides }
}

const meta: Meta<AccountPage> = {
  title: 'Account/AccountPage',
  component: AccountPage,
  decorators: [
    moduleMetadata({
      providers: [
        { provide: MeService, useValue: fakeMe() },
        { provide: MfaService, useValue: fakeMfa() },
        { provide: ApiTokensApplicationService, useValue: fakeTokens() },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<AccountPage>

/** Profile tab: username, member-since date, and the password-change form. */
export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText("Nom d'utilisateur : florian")).toBeInTheDocument())
    expect(canvas.queryByText('Statut : Super-administrateur')).not.toBeInTheDocument()
  },
}

export const AsSuperAdmin: Story = {
  decorators: [moduleMetadata({ providers: [{ provide: MeService, useValue: fakeMe(true) }] })],
  play: async ({ canvasElement }) => {
    await waitFor(() =>
      expect(within(canvasElement).getByText('Statut : Super-administrateur')).toBeInTheDocument(),
    )
  },
}

/** Confirming with a different password than just typed surfaces a mismatch error. */
export const PasswordMismatch: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(canvas.getByLabelText(/Nouveau mot de passe/), 'correct-horse-1')
    await userEvent.type(canvas.getByLabelText(/Confirmer le nouveau mot de passe/), 'different')
    await userEvent.tab()
    await waitFor(() =>
      expect(canvas.getByText('Les mots de passe ne correspondent pas.')).toBeInTheDocument(),
    )
  },
}

/** Filling in a valid, matching password and submitting resets the form. */
export const ChangingPassword: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(canvas.getByLabelText('Mot de passe actuel *'), 'old-password')
    await userEvent.type(canvas.getByLabelText(/Nouveau mot de passe/), 'correct-horse-1')
    await userEvent.type(
      canvas.getByLabelText(/Confirmer le nouveau mot de passe/),
      'correct-horse-1',
    )
    await userEvent.click(canvas.getByRole('button', { name: 'Changer le mot de passe' }))
    await waitFor(() => expect(canvas.getByLabelText('Mot de passe actuel *')).toHaveValue(''))
  },
}

/** The "Sécurité" tab shows MFA (disabled by default) and the passkeys list. */
export const SwitchingToTheSecurityTab: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('tab', { name: 'Sécurité' }))
    await waitFor(() =>
      expect(
        canvas.getByText("La double authentification n'est pas activée sur ce compte."),
      ).toBeInTheDocument(),
    )
    expect(canvas.getByText("Aucune clé d'accès enregistrée.")).toBeInTheDocument()
  },
}

/** Starting TOTP enrollment shows the QR code, secret, and confirmation field. */
export const EnrollingInTotp: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: MfaService,
          useValue: fakeMfa({
            enrollTotp: () =>
              of({ secret: 'ABCD EFGH IJKL', otpauth_url: 'otpauth://totp/ArtiFerris:florian' }),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('tab', { name: 'Sécurité' }))
    await userEvent.click(await canvas.findByRole('button', { name: 'Activer' }))
    await waitFor(() => expect(canvas.getByText('ABCD EFGH IJKL')).toBeInTheDocument())
    expect(canvas.getByRole('img', { name: "QR code d'activation" })).toBeInTheDocument()
  },
}

/** An account with MFA already enabled shows its backup-code count and management actions. */
export const AsMfaEnabledUser: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: MfaService,
          useValue: fakeMfa({
            getStatus: () => of(ENABLED_STATUS),
            listPasskeys: () => of([PASSKEY]),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('tab', { name: 'Sécurité' }))
    await waitFor(() =>
      expect(canvas.getByText('Codes de secours restants : 6')).toBeInTheDocument(),
    )
    expect(canvas.getByText('MacBook Touch ID')).toBeInTheDocument()
  },
}

/** The "Jetons API" tab lists existing tokens and lets you create a new one. */
export const CreatingAnApiToken: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: ApiTokensApplicationService,
          useValue: fakeTokens({
            list: () => of([TOKEN]),
            create: () => of({ id: 't2', token: 'artiferris_pat_abc123' }),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('tab', { name: 'Jetons API' }))
    await waitFor(() => expect(canvas.getByText('mon laptop')).toBeInTheDocument())
    await userEvent.click(canvas.getByRole('button', { name: 'Nouveau token' }))
    await userEvent.type(await canvas.findByLabelText(/Nom \(ex\. « mon laptop »\)/), 'CI runner')
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))
    await waitFor(() => expect(canvas.getByText('artiferris_pat_abc123')).toBeInTheDocument())
  },
}
