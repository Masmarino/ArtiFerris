import {
  applicationConfig,
  moduleMetadata,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { Component, inject, provideAppInitializer, signal } from '@angular/core'
import { provideLocationMocks } from '@angular/common/testing'
import { Router, provideRouter } from '@angular/router'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { of } from 'rxjs'
import { AccountPage } from './account-page'
import { MeService } from '../../shell/application/me.service'
import type { MfaPort, MfaStatus } from '@masmarino/gabarit/auth'
import { fakeAuthPort, withAuthPort } from '../../auth/kit/auth-story-helpers'
import { KitMfaAdapter } from '../../auth/kit/kit-mfa.adapter'
import { AuthService } from '../../auth/application/auth.service'
import { SessionRevocationService } from '../../auth/application/session-revocation.service'
import { ApiTokensApplicationService } from '../../tokens/application/api-tokens.application-service'
import type { ApiToken } from '../../tokens/domain/api-token.entity'

const PASSKEY_ONLY: MfaStatus = {
  totpEnabled: false,
  backupCodesRemaining: 10,
  passkeys: [
    {
      id: 'pk1',
      name: 'MacBook Touch ID',
      createdAt: '2026-06-01T00:00:00Z',
      lastUsedAt: '2026-10-01T08:00:00Z',
    },
  ],
}

const APP_AND_PASSKEY: MfaStatus = { ...PASSKEY_ONLY, totpEnabled: true, backupCodesRemaining: 6 }

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
    email: signal<string | null>('florian@corp.example'),
    changePassword: () => of(undefined),
  }
}

/** Gabarit's MFA port, as KitMfaAdapter serves it from our API. */
function fakeMfa(overrides: Partial<MfaPort> = {}): Partial<MfaPort> {
  return { status: () => of(PASSKEY_ONLY), ...overrides }
}

const withMfa = (port: Partial<MfaPort>) => ({ provide: KitMfaAdapter, useValue: port })

function fakeTokens(
  overrides: Partial<ApiTokensApplicationService> = {},
): Partial<ApiTokensApplicationService> {
  return { list: () => of([]), ...overrides }
}

const signOutAndRedirect = fn()

@Component({ standalone: true, template: '' })
class Blank {}

// In-memory navigation: the section links are relative to the route, so a click goes to `/?section=<key>`.
const withRouter = applicationConfig({
  providers: [provideRouter([{ path: '**', component: Blank }]), provideLocationMocks()],
})
const startAt = (url: string) =>
  applicationConfig({
    providers: [provideAppInitializer(() => inject(Router).navigateByUrl(url))],
  })

/** The section the side menu marks as the current one. */
async function expectSection(canvasElement: HTMLElement, name: string): Promise<void> {
  const nav = within(canvasElement).getByRole('navigation', { name: 'Réglages du compte' })
  await waitFor(() =>
    expect(within(nav).getByRole('link', { name })).toHaveAttribute('aria-current', 'page'),
  )
}

const meta: Meta<AccountPage> = {
  title: 'Account/AccountPage',
  component: AccountPage,
  parameters: { layout: 'fullscreen' },
  decorators: [
    withRouter,
    // The settings read the organisation's sign-in settings through KitAuthAdapter, provided at the root.
    applicationConfig({ providers: [withAuthPort(fakeAuthPort())] }),
    moduleMetadata({
      providers: [
        { provide: MeService, useValue: fakeMe() },
        withMfa(fakeMfa()),
        { provide: AuthService, useValue: { logoutEverywhere: () => of(undefined) } },
        { provide: ApiTokensApplicationService, useValue: fakeTokens() },
        { provide: SessionRevocationService, useValue: { signOutAndRedirect } },
      ],
    }),
  ],
  beforeEach: () => signOutAndRedirect.mockClear(),
}
export default meta

type Story = StoryObj<AccountPage>

export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('florian')).toBeInTheDocument())
    await expectSection(canvasElement, 'Profil')
    expect(canvas.getByText("Nom d'utilisateur, non modifiable")).toBeInTheDocument()
    expect(canvas.queryByText('Super-administrateur')).not.toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Déconnexion' })).toBeInTheDocument()
  },
}

export const AsSuperAdmin: Story = {
  decorators: [moduleMetadata({ providers: [{ provide: MeService, useValue: fakeMe(true) }] })],
  play: async ({ canvasElement }) => {
    await waitFor(() =>
      expect(within(canvasElement).getByText('Super-administrateur')).toBeInTheDocument(),
    )
  },
}

/** The side menu opens a section, and the address keeps it. */
export const OpeningASection: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('link', { name: 'Mot de passe' }))
    await expectSection(canvasElement, 'Mot de passe')
    expect(await canvas.findByLabelText('Mot de passe actuel *')).toBeInTheDocument()
  },
}

export const Password: Story = {
  decorators: [startAt('/?section=password')],
  play: async ({ canvasElement }) => {
    await expectSection(canvasElement, 'Mot de passe')
    expect(within(canvasElement).getByText('8 caractères minimum')).toBeInTheDocument()
  },
}

export const PasswordMismatch: Story = {
  decorators: [startAt('/?section=password')],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await canvas.findByLabelText(/Nouveau mot de passe/), 'correct-horse-1')
    await userEvent.type(canvas.getByLabelText(/Confirmer le nouveau mot de passe/), 'different')
    await userEvent.tab()
    await waitFor(() =>
      expect(
        canvas.getByText('Les nouveaux mots de passe ne correspondent pas.'),
      ).toBeInTheDocument(),
    )
  },
}

export const ChangingPassword: Story = {
  decorators: [startAt('/?section=password')],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await canvas.findByLabelText('Mot de passe actuel *'), 'old-password')
    await userEvent.type(canvas.getByLabelText(/Nouveau mot de passe/), 'correct-horse-1')
    await userEvent.type(
      canvas.getByLabelText(/Confirmer le nouveau mot de passe/),
      'correct-horse-1',
    )
    await userEvent.click(canvas.getByRole('button', { name: 'Changer le mot de passe' }))
    // The other sessions end; this one goes on.
    expect(await canvas.findByText('Mot de passe modifié.')).toBeInTheDocument()
    expect(signOutAndRedirect).not.toHaveBeenCalled()
    expect(canvas.getByLabelText('Mot de passe actuel *')).toHaveValue('')
  },
}

/** A passkey alone: its backup codes, and the app offered as an optional second way in. */
export const Security: Story = {
  decorators: [startAt('/?section=security')],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() =>
      expect(canvas.getByText('Aucune application configurée')).toBeInTheDocument(),
    )
    expect(canvas.getByText('10 codes de secours restants.')).toBeInTheDocument()
    expect(canvas.getByText('MacBook Touch ID')).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Se déconnecter partout' })).toBeInTheDocument()
  },
}

export const AsMfaEnabledUser: Story = {
  decorators: [
    startAt('/?section=security'),
    moduleMetadata({ providers: [withMfa(fakeMfa({ status: () => of(APP_AND_PASSKEY) }))] }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('Application configurée')).toBeInTheDocument())
    expect(canvas.getByText('6 codes de secours restants.')).toBeInTheDocument()
  },
}

export const CreatingAnApiToken: Story = {
  decorators: [
    startAt('/?section=tokens'),
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
    await waitFor(() => expect(canvas.getByText('mon laptop')).toBeInTheDocument())
    await userEvent.type(canvas.getByLabelText('Nom du jeton'), 'CI runner')
    await userEvent.click(canvas.getByRole('button', { name: 'Générer' }))
    await waitFor(() => expect(canvas.getByText('artiferris_pat_abc123')).toBeInTheDocument())
  },
}
