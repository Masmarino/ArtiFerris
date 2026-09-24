import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { of, throwError } from 'rxjs'
import { ApiTokensAdmin } from './api-tokens'
import { AdminApiTokensService } from '../application/admin-api-tokens.service'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'
import type { AdminApiToken } from '../domain/admin-api-token.entity'

const CI_TOKEN: AdminApiToken = {
  id: 'tok-1',
  user_id: 'u1',
  username: 'florian',
  label: 'ci-bot',
  created_at: '2026-03-01T09:30:00Z',
  last_used_at: '2026-03-20T14:00:00Z',
  revoked_at: null,
}
const UNUSED_TOKEN: AdminApiToken = {
  id: 'tok-2',
  user_id: 'u2',
  username: 'acme-admin',
  label: 'laptop',
  created_at: '2026-03-05T10:00:00Z',
  last_used_at: null,
  revoked_at: null,
}
const REVOKED_TOKEN: AdminApiToken = {
  id: 'tok-3',
  user_id: 'u2',
  username: 'acme-admin',
  label: 'old-deploy-key',
  created_at: '2026-01-10T08:00:00Z',
  last_used_at: '2026-02-01T08:00:00Z',
  revoked_at: '2026-02-15T12:00:00Z',
}

function fakeTokens(
  overrides: Partial<AdminApiTokensService> = {},
): Partial<AdminApiTokensService> {
  return {
    list: fn(() => of([CI_TOKEN, UNUSED_TOKEN, REVOKED_TOKEN])),
    revoke: fn(() => of(undefined)),
    ...overrides,
  }
}

const toast = { success: fn(), error: fn() }

/** Stands in for the confirmation dialog the component opens before revoking. */
function fakeConfirm(answer = true) {
  return { ask: fn(() => Promise.resolve(answer)) }
}

function withTokens(
  tokens: Partial<AdminApiTokensService>,
  confirm: ReturnType<typeof fakeConfirm> = fakeConfirm(),
) {
  return moduleMetadata({
    providers: [
      { provide: AdminApiTokensService, useValue: tokens },
      { provide: ConfirmService, useValue: confirm },
    ],
  })
}

const meta: Meta<ApiTokensAdmin> = {
  title: 'Admin/ApiTokensAdmin',
  component: ApiTokensAdmin,
  beforeEach: () => {
    toast.success.mockClear()
    toast.error.mockClear()
  },
  decorators: [
    moduleMetadata({
      providers: [
        { provide: AdminApiTokensService, useValue: fakeTokens() },
        { provide: ToastService, useValue: toast },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<ApiTokensAdmin>

/** Active tokens with a "Révoquer" action, and the revoked ones listed separately without one. */
export const ActiveAndRevoked: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('ci-bot')).toBeInTheDocument())
    expect(canvas.getByRole('heading', { name: 'Jetons actifs' })).toBeInTheDocument()
    expect(canvas.getByRole('heading', { name: 'Jetons révoqués' })).toBeInTheDocument()
    expect(canvas.getByText('laptop')).toBeInTheDocument()
    expect(canvas.getByText('old-deploy-key')).toBeInTheDocument()
    expect(canvas.getAllByRole('button', { name: 'Révoquer' })).toHaveLength(2)
    // The unused token has never authenticated anything.
    expect(
      within(canvas.getByRole('row', { name: /laptop/ })).getByText('Jamais'),
    ).toBeInTheDocument()
  },
}

export const NoTokens: Story = {
  decorators: [withTokens(fakeTokens({ list: () => of([]) }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('Aucun jeton actif')).toBeInTheDocument())
    expect(canvas.queryByRole('heading', { name: 'Jetons révoqués' })).not.toBeInTheDocument()
    expect(canvas.queryByRole('button', { name: 'Révoquer' })).not.toBeInTheDocument()
  },
}

/** Every token has been revoked: the active card falls back to its empty message. */
export const OnlyRevokedTokens: Story = {
  decorators: [withTokens(fakeTokens({ list: () => of([REVOKED_TOKEN]) }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('Aucun jeton actif')).toBeInTheDocument())
    expect(canvas.getByText('old-deploy-key')).toBeInTheDocument()
    expect(canvas.queryByRole('button', { name: 'Révoquer' })).not.toBeInTheDocument()
  },
}

const scopedTokens = fakeTokens()
/** Embedded in one organization's admin page: the list is scoped to it. */
export const ScopedToAnOrganization: Story = {
  args: { organizationId: 'org-acme' },
  decorators: [withTokens(scopedTokens)],
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(within(canvasElement).getByText('ci-bot')).toBeInTheDocument())
    expect(scopedTokens.list).toHaveBeenCalledWith('org-acme')
  },
}

// Stateful so the re-fetch after revoking returns the shortened list.
let ciBotRevoked = false
const revokingTokens = fakeTokens({
  list: fn(() => of(ciBotRevoked ? [UNUSED_TOKEN] : [CI_TOKEN, UNUSED_TOKEN])),
  revoke: fn(() => {
    ciBotRevoked = true
    return of(undefined)
  }),
})
const confirmRevoking = fakeConfirm(true)
export const RevokingAToken: Story = {
  beforeEach: () => {
    ciBotRevoked = false
  },
  decorators: [withTokens(revokingTokens, confirmRevoking)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('ci-bot')).toBeInTheDocument())
    await userEvent.click(within(canvas.getByRole('row', { name: /ci-bot/ })).getByRole('button'))

    await waitFor(() =>
      expect(confirmRevoking.ask).toHaveBeenCalledWith(
        expect.objectContaining({
          heading: 'Révoquer le jeton',
          message: 'Révoquer le jeton « ci-bot » de florian ?',
        }),
      ),
    )
    await waitFor(() => expect(revokingTokens.revoke).toHaveBeenCalledWith('tok-1'))
    expect(toast.success).toHaveBeenCalledWith('Jeton « ci-bot » révoqué.')
    // The list is re-fetched, so the revoked token disappears from the active table.
    await waitFor(() => expect(canvas.queryByText('ci-bot')).not.toBeInTheDocument())
    expect(canvas.getByText('laptop')).toBeInTheDocument()
  },
}

const cancelledTokens = fakeTokens()
const confirmCancelled = fakeConfirm(false)
export const RevokeCancelled: Story = {
  decorators: [withTokens(cancelledTokens, confirmCancelled)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('ci-bot')).toBeInTheDocument())
    await userEvent.click(within(canvas.getByRole('row', { name: /ci-bot/ })).getByRole('button'))

    await waitFor(() => expect(confirmCancelled.ask).toHaveBeenCalled())
    expect(cancelledTokens.revoke).not.toHaveBeenCalled()
    expect(toast.success).not.toHaveBeenCalled()
  },
}

const failingTokens = fakeTokens({ revoke: fn(() => throwError(() => new Error('boom'))) })
export const RevokeFailed: Story = {
  decorators: [withTokens(failingTokens)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('ci-bot')).toBeInTheDocument())
    await userEvent.click(within(canvas.getByRole('row', { name: /ci-bot/ })).getByRole('button'))

    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith('Échec de la révocation du jeton « ci-bot ».'),
    )
    expect(toast.success).not.toHaveBeenCalled()
    expect(canvas.getByText('ci-bot')).toBeInTheDocument()
  },
}

const fullPage = Array.from({ length: 500 }, (_, i) => ({
  ...UNUSED_TOKEN,
  id: `tok-page-${i}`,
  label: `key-${i}`,
}))
/** The server caps the list at 500 rows, so the admin is told it may be incomplete. */
export const ListTruncated: Story = {
  decorators: [withTokens(fakeTokens({ list: () => of(fullPage) }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() =>
      expect(canvas.getByText('La liste est limitée à 500 jetons.')).toBeInTheDocument(),
    )
  },
}
