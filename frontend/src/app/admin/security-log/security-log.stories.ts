import { signal } from '@angular/core'
import { HttpErrorResponse } from '@angular/common/http'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, spyOn, userEvent, waitFor, within } from 'storybook/test'
import { defer, NEVER, of, Subject, throwError } from 'rxjs'
import { SecurityLog } from './security-log'
import { AuditService } from '../application/audit.service'
import { OrganizationMembersService } from '../application/organization-members.service'
import { UsersService } from '../../users/application/users.service'
import { RepositoriesService } from '../../repositories/application/repositories.service'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'
import type { AuditEntry, AuditPage, BlockedAccount } from '../domain/audit.entity'
import type { OrganizationMember } from '../domain/organization-member.entity'
import type { UserSummary } from '../../users/domain/user.entity'
import type { RepositorySummary } from '../../repositories/domain/repository.entity'

const USERS: UserSummary[] = [
  {
    id: 'u1',
    username: 'florian',
    is_super_admin: true,
    organization_id: 'org-public',
    email: 'florian@example.com',
    invitation_pending: false,
  },
]

const MEMBERS: OrganizationMember[] = [
  {
    id: 'm1',
    username: 'acme-admin',
    email: 'admin@acme.example.com',
    is_organization_admin: true,
    invitation_pending: false,
  },
]

function repository(id: string, name: string): RepositorySummary {
  return {
    id,
    name,
    format: 'npm',
    repo_type: 'hosted',
    remote_url: null,
    remote_credentials_set: false,
    group_members: [],
    quota_bytes: null,
    retention_keep_last_n: null,
    is_public: false,
    my_role: 'admin',
    organization_id: 'org-public',
    owner_name: 'Public',
    owner_is_personal: false,
  }
}

const EVENTS: AuditEntry[] = [
  {
    aggregate_type: 'Security',
    aggregate_id: 's1',
    event_type: 'LoginFailed',
    payload: { username: 'mallory', ip: '203.0.113.7' },
    occurred_at: '2026-03-01T10:00:00Z',
    actor_id: null,
  },
  {
    aggregate_type: 'Security',
    aggregate_id: 's2',
    event_type: 'LoginFailed',
    payload: { username: 'mallory', ip: '203.0.113.8' },
    occurred_at: '2026-03-01T10:01:00Z',
    actor_id: null,
  },
  {
    aggregate_type: 'Security',
    aggregate_id: 's3',
    event_type: 'AccessDenied',
    payload: { repository_id: 'r1', action: 'push' },
    occurred_at: '2026-03-01T11:00:00Z',
    actor_id: 'u1',
  },
  {
    aggregate_type: 'Security',
    aggregate_id: 's4',
    event_type: 'PasswordChangeFailed',
    payload: { ip: '198.51.100.2' },
    occurred_at: '2026-03-01T12:00:00Z',
    actor_id: 'u-unknown',
  },
  {
    aggregate_type: 'Security',
    aggregate_id: 's5',
    event_type: 'AccountBlocked',
    payload: {},
    occurred_at: '2026-03-01T13:00:00Z',
    actor_id: null,
  },
]

const BLOCKED: BlockedAccount[] = [
  { username: 'mallory', remaining_seconds: 600 },
  { username: 'eve', remaining_seconds: 20 },
]

const page = (entries: AuditEntry[], next_cursor: string | null = null): AuditPage => ({
  entries,
  next_cursor,
})

function fakeAudit(overrides: Partial<AuditService> = {}): Partial<AuditService> {
  return { query: () => of(page(EVENTS)), blockedAccounts: () => of(BLOCKED), ...overrides }
}
function fakeUsers(overrides: Partial<UsersService> = {}): Partial<UsersService> {
  return { list: () => of(USERS), ...overrides }
}
function fakeMembers(
  overrides: Partial<OrganizationMembersService> = {},
): Partial<OrganizationMembersService> {
  return { list: () => of(MEMBERS), ...overrides }
}
function fakeRepositories(
  overrides: Partial<RepositoriesService> = {},
): Partial<RepositoriesService> {
  return { list: () => of([repository('r1', 'acme-npm')]), ...overrides }
}

const meta: Meta<SecurityLog> = {
  title: 'Admin/SecurityLog',
  component: SecurityLog,
  decorators: [
    moduleMetadata({
      providers: [
        { provide: AuditService, useValue: fakeAudit() },
        { provide: UsersService, useValue: fakeUsers() },
        { provide: OrganizationMembersService, useValue: fakeMembers() },
        { provide: RepositoriesService, useValue: fakeRepositories() },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<SecurityLog>

function rowOf(table: HTMLElement, text: string): HTMLElement {
  const row = within(table)
    .getAllByRole('row')
    .find((r) => r.textContent?.includes(text))
  if (!row) {
    throw new Error(`no row containing "${text}"`)
  }
  return row
}

/** Super-admin view: actor names and repository names resolved, blocked accounts listed. */
export const Populated: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const table = await canvas.findByRole('table', { name: 'Journal de sécurité' })
    expect(within(table).getAllByRole('row')).toHaveLength(EVENTS.length + 1)

    const summary = canvas.getByRole('table', { name: 'Événements par type' })
    const first = within(summary).getAllByRole('row')[1]
    expect(first).toHaveTextContent('Échec de connexion')
    expect(first).toHaveTextContent('2')

    expect(canvas.getByRole('button', { name: 'Télécharger en CSV' })).toBeEnabled()
  },
}

/** Actor comes from the payload username first, then the users lookup, then the raw id, then a dash. */
export const ActorResolution: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const table = await canvas.findByRole('table', { name: 'Journal de sécurité' })
    await waitFor(() => expect(rowOf(table, 'Accès refusé')).toHaveTextContent('florian'))
    expect(rowOf(table, '203.0.113.7')).toHaveTextContent('mallory')
    expect(rowOf(table, 'Échec de changement de mot de passe')).toHaveTextContent('u-unknown')
    expect(rowOf(table, 'AccountBlocked')).toHaveTextContent('—')
  },
}

/** Details column: source IP for failed logins/password changes, action and repository for denied access. */
export const EventDetails: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const table = await canvas.findByRole('table', { name: 'Journal de sécurité' })
    expect(rowOf(table, '203.0.113.7')).toHaveTextContent('Depuis 203.0.113.7')
    expect(rowOf(table, 'Échec de changement de mot de passe')).toHaveTextContent(
      'Depuis 198.51.100.2',
    )
    await waitFor(() =>
      expect(rowOf(table, 'Accès refusé')).toHaveTextContent(
        'Action « push » refusée sur acme-npm',
      ),
    )
  },
}

/** Lookups for names failing must not break the page: raw ids are shown instead. */
export const NameLookupsFailed: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: UsersService,
          useValue: fakeUsers({ list: () => throwError(() => new Error('users down')) }),
        },
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({ list: () => throwError(() => new Error('repos down')) }),
        },
        {
          provide: AuditService,
          useValue: fakeAudit({ blockedAccounts: () => throwError(() => new Error('down')) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const table = await canvas.findByRole('table', { name: 'Journal de sécurité' })
    const denied = rowOf(table, 'Accès refusé')
    expect(denied).toHaveTextContent('u1')
    expect(denied).toHaveTextContent('Action « push » refusée sur r1')
    expect(canvas.queryByText('Comptes actuellement bloqués')).not.toBeInTheDocument()
  },
}

/** Blocked accounts: whole minutes rounded up, and "moins d'une minute" under a minute. */
export const BlockedAccounts: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Comptes actuellement bloqués')).toBeInTheDocument()
    expect(canvas.getByText('mallory — débloqué dans 10 minutes')).toBeInTheDocument()
    expect(canvas.getByText("eve — débloqué dans moins d'une minute")).toBeInTheDocument()
  },
}

const ACCOUNT_EVENTS: AuditEntry[] = [
  {
    aggregate_type: 'Security',
    aggregate_id: 'c1',
    event_type: 'LoginSucceeded',
    payload: { method: 'password', second_factor: 'totp', user_id: 'u1' },
    occurred_at: '2026-03-04T09:00:00Z',
    actor_id: 'u1',
  },
  {
    aggregate_type: 'Security',
    aggregate_id: 'c2',
    event_type: 'LoginSucceeded',
    payload: { method: 'oidc', second_factor: null, user_id: 'u1' },
    occurred_at: '2026-03-04T09:05:00Z',
    actor_id: 'u1',
  },
  {
    aggregate_type: 'Security',
    aggregate_id: 'c3',
    event_type: 'PasswordChanged',
    payload: { user_id: 'u1' },
    occurred_at: '2026-03-04T09:10:00Z',
    actor_id: 'u1',
  },
  {
    aggregate_type: 'Security',
    aggregate_id: 'c4',
    event_type: 'MfaEnabled',
    payload: { method: 'totp', user_id: 'u1' },
    occurred_at: '2026-03-04T09:15:00Z',
    actor_id: 'u1',
  },
  {
    aggregate_type: 'Security',
    aggregate_id: 'c5',
    event_type: 'ApiTokenCreated',
    payload: { label: 'ci-deploy', token_id: 't1', user_id: 'u1' },
    occurred_at: '2026-03-04T09:20:00Z',
    actor_id: 'u1',
  },
  {
    aggregate_type: 'Security',
    aggregate_id: 'c6',
    event_type: 'ApiTokenRevoked',
    payload: { token_id: 't1', user_id: 'u1' },
    occurred_at: '2026-03-04T09:25:00Z',
    actor_id: 'u1',
  },
  {
    aggregate_type: 'Security',
    aggregate_id: 'c7',
    event_type: 'SessionsRevoked',
    payload: { user_id: 'u1' },
    occurred_at: '2026-03-04T09:30:00Z',
    actor_id: 'u1',
  },
  {
    aggregate_type: 'Security',
    aggregate_id: 'c8',
    event_type: 'PasskeyAdded',
    payload: { passkey_id: 'k1', user_id: 'u1' },
    occurred_at: '2026-03-04T09:35:00Z',
    actor_id: 'u1',
  },
]

/** Successful logins and credential changes read in French, next to the failures. */
export const AccountEvents: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        { provide: AuditService, useValue: fakeAudit({ query: () => of(page(ACCOUNT_EVENTS)) }) },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const table = await within(canvasElement).findByRole('table', { name: 'Journal de sécurité' })
    expect(within(table).getAllByText('Connexion réussie')).toHaveLength(2)
    expect(within(table).getByText('Via mot de passe + application TOTP')).toBeInTheDocument()
    expect(within(table).getByText('Via SSO (OIDC)')).toBeInTheDocument()
    expect(within(table).getByText('Mot de passe modifié')).toBeInTheDocument()
    expect(within(table).getByText('Méthode : application TOTP')).toBeInTheDocument()
    expect(within(table).getByText("Jeton d'API créé")).toBeInTheDocument()
    expect(within(table).getByText('Libellé : ci-deploy')).toBeInTheDocument()
    expect(within(table).getByText("Jeton d'API révoqué")).toBeInTheDocument()
    expect(within(table).getByText('Déconnexion de toutes les sessions')).toBeInTheDocument()
    expect(within(table).getByText("Clé d'accès ajoutée")).toBeInTheDocument()
    await waitFor(() => expect(within(table).getAllByText('florian').length).toBeGreaterThan(0))
  },
}

const pagedQuery = fn<AuditService['query']>((filter) =>
  of(filter?.cursor === 'page-2' ? page([EVENTS[4]], null) : page(EVENTS.slice(0, 4), 'page-2')),
)

/** The instance-level log (super-admin, no organization) pages the same way, failed logins included. */
export const LoadMore: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuditService,
          useValue: { query: pagedQuery, blockedAccounts: () => of(BLOCKED) },
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const table = await canvas.findByRole('table', { name: 'Journal de sécurité' })
    expect(within(table).getAllByRole('row')).toHaveLength(5)

    await userEvent.click(canvas.getByRole('button', { name: 'Charger plus' }))

    await waitFor(() => expect(within(table).getAllByRole('row')).toHaveLength(6))
    expect(pagedQuery).toHaveBeenLastCalledWith({
      aggregate_type: 'Security',
      organization_id: undefined,
      cursor: 'page-2',
    })
    expect(canvas.queryByRole('button', { name: 'Charger plus' })).not.toBeInTheDocument()
  },
}

/** While older events exist, the summary and the export say what they cover. */
export const PartialCoverageNotes: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuditService,
          useValue: fakeAudit({ query: () => of(page(EVENTS.slice(0, 4), 'page-2')) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText(/Calculé sur les 4 événements chargés/)).toBeInTheDocument()
    expect(canvas.getByText(/l'export récupère aussi les suivants/)).toBeInTheDocument()
  },
}

/** With events still on the server, the export pages through all of them. */
export const CsvDownloadFetchesEveryPage: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuditService,
          useValue: { query: pagedQuery, blockedAccounts: () => of(BLOCKED) },
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await canvas.findByRole('table', { name: 'Journal de sécurité' })
    const createObjectURL = spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock')
    const revokeObjectURL = spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
    const click = spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined)
    try {
      await userEvent.click(canvas.getByRole('button', { name: 'Télécharger en CSV' }))

      await waitFor(() => expect(createObjectURL).toHaveBeenCalledTimes(1))
      const lines = (await (createObjectURL.mock.calls[0][0] as Blob).text()).split('\r\n')
      expect(lines).toHaveLength(EVENTS.length + 1)
    } finally {
      createObjectURL.mockRestore()
      revokeObjectURL.mockRestore()
      click.mockRestore()
    }
  },
}

/** A slow export shows its progress and can be cancelled without producing a file. */
export const CsvExportInProgress: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuditService,
          useValue: fakeAudit({
            query: (filter) => (filter?.cursor ? NEVER : of(page(EVENTS.slice(0, 4), 'page-2'))),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const createObjectURL = spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock')
    try {
      await userEvent.click(await canvas.findByRole('button', { name: 'Télécharger en CSV' }))
      expect(await canvas.findByRole('status')).toHaveTextContent(
        'Export en cours… 4 événements récupérés',
      )

      await userEvent.click(canvas.getByRole('button', { name: 'Annuler' }))

      expect(await canvas.findByRole('button', { name: 'Télécharger en CSV' })).toBeEnabled()
      expect(createObjectURL).not.toHaveBeenCalled()
    } finally {
      createObjectURL.mockRestore()
    }
  },
}

/** A failed next page keeps the rows on screen and lets the reader retry. */
export const LoadMoreFailed: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuditService,
          useValue: fakeAudit({
            query: (filter) =>
              filter?.cursor
                ? throwError(() => new Error('network error'))
                : of(page(EVENTS, 'page-2')),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Charger plus' }))
    expect(await canvas.findByRole('alert')).toHaveTextContent(
      'Impossible de charger la suite du journal. Réessayez.',
    )
    expect(canvas.getByRole('table', { name: 'Journal de sécurité' })).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Charger plus' })).toBeEnabled()
  },
}

const UNLOCK_KEYS: BlockedAccount[] = [
  { username: 'login-user:alice', remaining_seconds: 600 },
  { username: 'login-org-user:0b8f6a58-1c3e-4a7b-9d2f-5e6a7b8c9d0e:bob', remaining_seconds: 300 },
  { username: 'mfa:3f2b1c00-0000-4000-8000-000000000001', remaining_seconds: 120 },
]

const unlockToast = { success: fn(), error: fn() }

function withUnlock(audit: Partial<AuditService>, confirmed = true) {
  return moduleMetadata({
    providers: [
      { provide: AuditService, useValue: fakeAudit(audit) },
      { provide: ConfirmService, useValue: { ask: fn(() => Promise.resolve(confirmed)) } },
      { provide: ToastService, useValue: unlockToast },
    ],
  })
}

/** Login keys show the username and get an unlock button; an MFA key shows raw and gets none. */
export const BlockedKeysAndUnlockButtons: Story = {
  decorators: [withUnlock({ blockedAccounts: () => of(UNLOCK_KEYS) })],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText(/^alice — débloqué dans 10 minutes/)).toBeInTheDocument()
    expect(canvas.getByText(/^bob \(organisation\) — débloqué dans 5 minutes/)).toBeInTheDocument()
    expect(canvas.getByText(/^mfa:3f2b1c00/)).toBeInTheDocument()
    expect(canvas.getAllByRole('button', { name: 'Débloquer' })).toHaveLength(2)
  },
}

let aliceUnlocked = false
const unlockUsername = fn(() => {
  aliceUnlocked = true
  return of(undefined)
})
/** Confirming unlocks by the bare username and re-fetches the list, which no longer has the account. */
export const UnlockingAnAccount: Story = {
  beforeEach: () => {
    aliceUnlocked = false
    unlockToast.success.mockClear()
    unlockUsername.mockClear()
  },
  decorators: [
    withUnlock({
      blockedAccounts: () => of(aliceUnlocked ? UNLOCK_KEYS.slice(1) : UNLOCK_KEYS),
      unlockUsername,
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await canvas.findByText(/^alice — débloqué/)
    await userEvent.click(canvas.getAllByRole('button', { name: 'Débloquer' })[0])

    await waitFor(() => expect(unlockUsername).toHaveBeenCalledWith('alice'))
    await waitFor(() => expect(canvas.queryByText(/^alice — débloqué/)).not.toBeInTheDocument())
    expect(unlockToast.success).toHaveBeenCalledWith('alice a été débloqué·e.')
  },
}

export const UnlockCancelled: Story = {
  decorators: [
    withUnlock(
      { blockedAccounts: () => of(UNLOCK_KEYS), unlockUsername: fn(() => of(undefined)) },
      false,
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await canvas.findByText(/^alice — débloqué/)
    await userEvent.click(canvas.getAllByRole('button', { name: 'Débloquer' })[0])

    await waitFor(() =>
      expect(canvas.getAllByRole('button', { name: 'Débloquer' })).toHaveLength(2),
    )
    expect(unlockToast.success).not.toHaveBeenCalled()
    expect(unlockToast.error).not.toHaveBeenCalled()
  },
}

/** The backend refuses (403) or does not know the account (404): a French toast, and the row stays. */
export const UnlockForbidden: Story = {
  beforeEach: () => unlockToast.error.mockClear(),
  decorators: [
    withUnlock({
      blockedAccounts: () => of(UNLOCK_KEYS),
      unlockUsername: () => throwError(() => new HttpErrorResponse({ status: 403 })),
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await canvas.findByText(/^alice — débloqué/)
    await userEvent.click(canvas.getAllByRole('button', { name: 'Débloquer' })[0])

    await waitFor(() =>
      expect(unlockToast.error).toHaveBeenCalledWith(
        "Vous n'avez pas le droit de débloquer alice.",
      ),
    )
    expect(canvas.getByText(/^alice — débloqué/)).toBeInTheDocument()
  },
}

export const NoBlockedAccounts: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        { provide: AuditService, useValue: fakeAudit({ blockedAccounts: () => of([]) }) },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await canvas.findByRole('table', { name: 'Journal de sécurité' })
    expect(canvas.queryByText('Comptes actuellement bloqués')).not.toBeInTheDocument()
  },
}

export const Empty: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuditService,
          useValue: fakeAudit({ query: () => of(page([])), blockedAccounts: () => of([]) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Aucun événement')).toBeInTheDocument()
    expect(canvas.queryByText('Résumé')).not.toBeInTheDocument()
    expect(canvas.queryByRole('table')).not.toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Télécharger en CSV' })).toBeDisabled()
  },
}

export const Loading: Story = {
  decorators: [
    moduleMetadata({
      providers: [{ provide: AuditService, useValue: fakeAudit({ query: () => NEVER }) }],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('status')).toHaveTextContent('Chargement…')
    expect(canvas.queryByRole('table')).not.toBeInTheDocument()
  },
}

export const LoadFailed: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuditService,
          useValue: fakeAudit({ query: () => throwError(() => new Error('network error')) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(
      await canvas.findByText(
        'Impossible de charger le journal de sécurité pour le moment. Réessayez plus tard.',
      ),
    ).toBeInTheDocument()
    expect(canvas.queryByRole('table')).not.toBeInTheDocument()
  },
}

const scopedQuery = fn<AuditService['query']>(() => of(page(EVENTS)))
const scopedBlocked = fn<AuditService['blockedAccounts']>(() => of(BLOCKED))
const scopedUsers = fn<UsersService['list']>(() => of(USERS))
const scopedMembers = fn<OrganizationMembersService['list']>(() => of(MEMBERS))

/** Embedded in an organization's admin page: scoped query, member names, and no blocked-accounts panel. */
export const ScopedToOrganization: Story = {
  args: { organizationId: 'org-acme' },
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuditService,
          useValue: { query: scopedQuery, blockedAccounts: scopedBlocked },
        },
        { provide: UsersService, useValue: { list: scopedUsers } },
        { provide: OrganizationMembersService, useValue: { list: scopedMembers } },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const table = await canvas.findByRole('table', { name: 'Journal de sécurité' })
    expect(scopedQuery).toHaveBeenLastCalledWith({
      aggregate_type: 'Security',
      organization_id: 'org-acme',
    })
    expect(scopedMembers).toHaveBeenLastCalledWith('org-acme')
    expect(scopedUsers).not.toHaveBeenCalled()
    expect(scopedBlocked).not.toHaveBeenCalled()
    expect(canvas.queryByText('Comptes actuellement bloqués')).not.toBeInTheDocument()
    expect(within(table).getAllByRole('row')).toHaveLength(EVENTS.length + 1)
  },
}

/** Members lookup is the only source of names in an organization: the actor id maps to a member username. */
export const ScopedActorFromMembers: Story = {
  args: { organizationId: 'org-acme' },
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuditService,
          useValue: fakeAudit({
            query: () => of(page([{ ...EVENTS[2], actor_id: 'm1' }])),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const table = await canvas.findByRole('table', { name: 'Journal de sécurité' })
    await waitFor(() => expect(rowOf(table, 'Accès refusé')).toHaveTextContent('acme-admin'))
  },
}

/** Exports the rows on screen, with resolved actor and details columns. */
export const CsvDownload: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await canvas.findByRole('table', { name: 'Journal de sécurité' })
    await waitFor(() =>
      expect(canvas.getByText(/Action « push » refusée sur acme-npm/)).toBeInTheDocument(),
    )
    const createObjectURL = spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock')
    const revokeObjectURL = spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
    const click = spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined)
    try {
      await userEvent.click(canvas.getByRole('button', { name: 'Télécharger en CSV' }))

      expect(createObjectURL).toHaveBeenCalledTimes(1)
      const blob = createObjectURL.mock.calls[0][0] as Blob
      expect(blob.type).toContain('text/csv')
      const lines = (await blob.text()).split('\r\n')
      expect(lines[0]).toBe('Date;Événement;Utilisateur;Détails')
      expect(lines).toHaveLength(EVENTS.length + 1)
      expect(lines[1]).toContain('Échec de connexion;mallory;Depuis 203.0.113.7')
      expect(lines[3]).toContain('florian')

      const anchor = click.mock.contexts[0] as HTMLAnchorElement
      expect(anchor.download).toMatch(/^artiferris-security-\d{4}-\d{2}-\d{2}\.csv$/)
      // Revoked about a second after the click.
      await waitFor(() => expect(revokeObjectURL).toHaveBeenCalledWith('blob:mock'), {
        timeout: 3000,
      })
    } finally {
      createObjectURL.mockRestore()
      revokeObjectURL.mockRestore()
      click.mockRestore()
    }
  },
}

let slowOrgA = new Subject<AuditPage>()

const ORG_B_EVENTS: AuditEntry[] = [
  {
    aggregate_type: 'Security',
    aggregate_id: 's9',
    event_type: 'MfaDisabled',
    payload: { username: 'bob' },
    occurred_at: '2026-03-02T09:00:00Z',
    actor_id: null,
  },
]

/** Switching organization while the first request is still in flight: its late response must not overwrite the new organization's rows. */
export const SwitchOrganizationWhileLoading: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuditService,
          useValue: fakeAudit({
            query: (filter) =>
              filter?.organization_id === 'org-a'
                ? defer(() => (slowOrgA = new Subject<AuditPage>()))
                : of(page(ORG_B_EVENTS)),
          }),
        },
      ],
    }),
  ],
  render: () => ({
    props: { organizationId: signal('org-a') },
    template: `
      <button type="button" (click)="organizationId.set('org-b')">Changer d'organisation</button>
      <app-security-log [organizationId]="organizationId()" />
    `,
    moduleMetadata: { imports: [SecurityLog] },
  }),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('status')).toHaveTextContent('Chargement…')

    await userEvent.click(canvas.getByRole('button', { name: "Changer d'organisation" }))
    const table = await canvas.findByRole('table', { name: 'Journal de sécurité' })
    expect(within(table).getByText('Double authentification désactivée')).toBeInTheDocument()

    slowOrgA.next(page(EVENTS))
    slowOrgA.complete()
    await new Promise((resolve) => setTimeout(resolve, 50))
    expect(within(table).getByText('Double authentification désactivée')).toBeInTheDocument()
    expect(canvas.queryByText('Échec de connexion')).not.toBeInTheDocument()
  },
}
