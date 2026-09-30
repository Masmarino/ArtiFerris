import { signal } from '@angular/core'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, spyOn, userEvent, waitFor, within } from 'storybook/test'
import { defer, NEVER, of, Subject, throwError } from 'rxjs'
import { AuditLog } from './audit-log'
import { AuditService } from '../application/audit.service'
import type { AuditEntry, AuditPage } from '../domain/audit.entity'

const ENTRIES: AuditEntry[] = [
  {
    aggregate_type: 'Permission',
    aggregate_id: 'repo-1',
    event_type: 'PermissionGranted',
    payload: { role: 'write' },
    occurred_at: '2026-03-01T10:00:00Z',
    actor_id: 'u1',
  },
  {
    aggregate_type: 'Permission',
    aggregate_id: 'repo-2',
    event_type: 'PermissionRevoked',
    payload: {},
    occurred_at: '2026-03-01T11:00:00Z',
    actor_id: 'u1',
  },
  {
    aggregate_type: 'PackageRepository',
    aggregate_id: 'repo-3',
    event_type: 'RepositoryCreated',
    payload: { name: 'acme-npm' },
    occurred_at: '2026-03-01T12:00:00Z',
    actor_id: null,
  },
]

const ORG_B_ENTRIES: AuditEntry[] = [
  {
    aggregate_type: 'User',
    aggregate_id: 'u9',
    event_type: 'UserInvited',
    payload: {},
    occurred_at: '2026-03-02T09:00:00Z',
    actor_id: 'u2',
  },
]

const page = (entries: AuditEntry[], next_cursor: string | null = null): AuditPage => ({
  entries,
  next_cursor,
})

function fakeAudit(overrides: Partial<AuditService> = {}): Partial<AuditService> {
  return { query: () => of(page(ENTRIES)), ...overrides }
}

const meta: Meta<AuditLog> = {
  title: 'Admin/AuditLog',
  component: AuditLog,
  decorators: [moduleMetadata({ providers: [{ provide: AuditService, useValue: fakeAudit() }] })],
}
export default meta

type Story = StoryObj<AuditLog>

export const Populated: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const table = await canvas.findByRole('table', { name: "Journal d'audit" })
    expect(within(table).getByText('PermissionGranted')).toBeInTheDocument()
    expect(within(table).getByText('RepositoryCreated')).toBeInTheDocument()
    expect(within(table).getAllByRole('row')).toHaveLength(ENTRIES.length + 1)

    const summary = canvas.getByRole('table', { name: 'Entrées par type' })
    const labels = within(summary)
      .getAllByRole('rowheader')
      .map((el) => el.textContent?.trim())
    expect(labels).toEqual(['Permission', 'PackageRepository'])
    expect(canvas.getByRole('button', { name: 'Télécharger en CSV' })).toBeEnabled()
  },
}

const ADMIN_EVENTS: AuditEntry[] = [
  {
    aggregate_type: 'Admin',
    aggregate_id: 'a1',
    event_type: 'IdentityProviderSet',
    payload: {
      before: { provider: 'oidc', issuer_url: 'https://idp.acme.example', client_id: 'artiferris' },
      after: { provider: 'oidc', issuer_url: 'https://idp.other.example', client_id: 'artiferris' },
      secret_changed: true,
    },
    occurred_at: '2026-03-03T10:00:00Z',
    actor_id: 'u1',
  },
  {
    aggregate_type: 'Admin',
    aggregate_id: 'a2',
    event_type: 'SuperAdminGranted',
    payload: { user_id: 'u5' },
    occurred_at: '2026-03-03T10:05:00Z',
    actor_id: 'u1',
  },
  {
    aggregate_type: 'Admin',
    aggregate_id: 'a3',
    event_type: 'SystemSettingsChanged',
    payload: { changes: [{ setting: 'registration_enabled', before: true, after: false }] },
    occurred_at: '2026-03-03T10:10:00Z',
    actor_id: 'u1',
  },
  {
    aggregate_type: 'Admin',
    aggregate_id: 'a4',
    event_type: 'ConfigurationExported',
    payload: { users: 12, repositories: 4, permissions: 30 },
    occurred_at: '2026-03-03T10:15:00Z',
    actor_id: 'u1',
  },
  {
    aggregate_type: 'Admin',
    aggregate_id: 'a5',
    event_type: 'UserInvited',
    payload: { username: 'alice', is_organization_admin: true, is_super_admin: false },
    occurred_at: '2026-03-03T10:20:00Z',
    actor_id: 'u1',
  },
]

export const AdministrativeEvents: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        { provide: AuditService, useValue: fakeAudit({ query: () => of(page(ADMIN_EVENTS)) }) },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const table = await within(canvasElement).findByRole('table', { name: "Journal d'audit" })
    expect(within(table).getByText("Fournisseur d'identité modifié")).toBeInTheDocument()
    expect(
      within(table).getByText(
        'OIDC https://idp.acme.example (client artiferris) → OIDC https://idp.other.example (client artiferris), secret modifié',
      ),
    ).toBeInTheDocument()
    expect(within(table).getByText('Super-administrateur accordé')).toBeInTheDocument()
    expect(within(table).getByText('Inscription ouverte : oui → non')).toBeInTheDocument()
    expect(within(table).getByText('12 utilisateurs, 4 dépôts, 30 permissions')).toBeInTheDocument()
    expect(within(table).getByText("alice (administrateur d'organisation)")).toBeInTheDocument()
    expect(within(table).queryByText('IdentityProviderSet')).not.toBeInTheDocument()
  },
}

const pagedQuery = fn<AuditService['query']>((filter) =>
  of(filter?.cursor === 'page-2' ? page([ENTRIES[2]], null) : page(ENTRIES.slice(0, 2), 'page-2')),
)

export const LoadMore: Story = {
  decorators: [
    moduleMetadata({ providers: [{ provide: AuditService, useValue: { query: pagedQuery } }] }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const table = await canvas.findByRole('table', { name: "Journal d'audit" })
    expect(within(table).getAllByRole('row')).toHaveLength(3)

    await userEvent.click(canvas.getByRole('button', { name: 'Charger plus' }))

    await waitFor(() => expect(within(table).getAllByRole('row')).toHaveLength(4))
    expect(pagedQuery).toHaveBeenLastCalledWith({
      exclude_aggregate_type: 'Security',
      organization_id: undefined,
      cursor: 'page-2',
    })
    expect(canvas.queryByRole('button', { name: 'Charger plus' })).not.toBeInTheDocument()
  },
}

export const LoadingMore: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuditService,
          useValue: fakeAudit({
            query: (filter) => (filter?.cursor ? NEVER : of(page(ENTRIES, 'page-2'))),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Charger plus' }))
    expect(await canvas.findByRole('button', { name: 'Chargement…' })).toBeDisabled()
    expect(canvas.getByRole('table', { name: "Journal d'audit" })).toBeInTheDocument()
  },
}

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
                : of(page(ENTRIES, 'page-2')),
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
    expect(canvas.getByRole('table', { name: "Journal d'audit" })).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Charger plus' })).toBeEnabled()
  },
}

export const Empty: Story = {
  decorators: [
    moduleMetadata({
      providers: [{ provide: AuditService, useValue: fakeAudit({ query: () => of(page([])) }) }],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Aucune entrée')).toBeInTheDocument()
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
        "Impossible de charger le journal d'audit pour le moment. Réessayez plus tard.",
      ),
    ).toBeInTheDocument()
    expect(canvas.queryByRole('table')).not.toBeInTheDocument()
  },
}

const scopedQuery = fn<AuditService['query']>(() => of(page(ENTRIES)))

export const ScopedToOrganization: Story = {
  args: { organizationId: 'org-acme' },
  decorators: [
    moduleMetadata({ providers: [{ provide: AuditService, useValue: { query: scopedQuery } }] }),
  ],
  play: async ({ canvasElement }) => {
    await within(canvasElement).findByRole('table', { name: "Journal d'audit" })
    expect(scopedQuery).toHaveBeenLastCalledWith({
      exclude_aggregate_type: 'Security',
      organization_id: 'org-acme',
    })
  },
}

export const Unscoped: Story = {
  decorators: [
    moduleMetadata({ providers: [{ provide: AuditService, useValue: { query: scopedQuery } }] }),
  ],
  play: async ({ canvasElement }) => {
    await within(canvasElement).findByRole('table', { name: "Journal d'audit" })
    expect(scopedQuery).toHaveBeenLastCalledWith({
      exclude_aggregate_type: 'Security',
      organization_id: undefined,
    })
  },
}

export const CsvDownload: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const createObjectURL = spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock')
    const revokeObjectURL = spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
    const click = spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined)
    try {
      await userEvent.click(await canvas.findByRole('button', { name: 'Télécharger en CSV' }))

      expect(createObjectURL).toHaveBeenCalledTimes(1)
      const blob = createObjectURL.mock.calls[0][0] as Blob
      expect(blob.type).toContain('text/csv')
      const lines = (await blob.text()).split('\r\n')
      expect(lines[0]).toBe('Date;Type;Identifiant;Événement;Acteur;Détails')
      expect(lines).toHaveLength(ENTRIES.length + 1)
      expect(lines[1]).toContain('PermissionGranted')
      expect(lines[1]).toContain('""role"":""write""')

      const anchor = click.mock.contexts[0] as HTMLAnchorElement
      expect(anchor.download).toMatch(/^artiferris-audit-\d{4}-\d{2}-\d{2}\.csv$/)
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

export const PartialCoverageNotes: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuditService,
          useValue: fakeAudit({ query: () => of(page(ENTRIES, 'page-2')) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText(/Calculé sur les 3 entrées chargées/)).toBeInTheDocument()
    expect(
      canvas.getByText(/3 entrées chargées ; l'export récupère aussi les suivantes/),
    ).toBeInTheDocument()
  },
}

const exportQuery = fn<AuditService['query']>((filter) =>
  of(filter?.cursor === 'page-2' ? page([ENTRIES[2]], null) : page(ENTRIES.slice(0, 2), 'page-2')),
)

export const CsvDownloadFetchesEveryPage: Story = {
  decorators: [
    moduleMetadata({ providers: [{ provide: AuditService, useValue: { query: exportQuery } }] }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const createObjectURL = spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock')
    const revokeObjectURL = spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
    const click = spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined)
    try {
      await userEvent.click(await canvas.findByRole('button', { name: 'Télécharger en CSV' }))

      await waitFor(() => expect(createObjectURL).toHaveBeenCalledTimes(1))
      const lines = (await (createObjectURL.mock.calls[0][0] as Blob).text()).split('\r\n')
      expect(lines).toHaveLength(ENTRIES.length + 1)
      expect(lines[3]).toContain('RepositoryCreated')
      expect(canvas.queryByText(/Export en cours/)).not.toBeInTheDocument()
    } finally {
      createObjectURL.mockRestore()
      revokeObjectURL.mockRestore()
      click.mockRestore()
    }
  },
}

export const CsvExportInProgress: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuditService,
          useValue: fakeAudit({
            query: (filter) => (filter?.cursor ? NEVER : of(page(ENTRIES, 'page-2'))),
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
        'Export en cours… 3 entrées récupérées',
      )

      await userEvent.click(canvas.getByRole('button', { name: 'Annuler' }))

      expect(await canvas.findByRole('button', { name: 'Télécharger en CSV' })).toBeEnabled()
      expect(createObjectURL).not.toHaveBeenCalled()
    } finally {
      createObjectURL.mockRestore()
    }
  },
}

export const CsvExportFailed: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuditService,
          useValue: fakeAudit({
            query: (filter) =>
              filter?.cursor
                ? throwError(() => new Error('network error'))
                : of(page(ENTRIES, 'page-2')),
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

      expect(await canvas.findByRole('alert')).toHaveTextContent("L'export a échoué")
      expect(createObjectURL).not.toHaveBeenCalled()
    } finally {
      createObjectURL.mockRestore()
    }
  },
}

let slowOrgA = new Subject<AuditPage>()

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
                : of(page(ORG_B_ENTRIES)),
          }),
        },
      ],
    }),
  ],
  render: () => ({
    props: { organizationId: signal('org-a') },
    template: `
      <button type="button" (click)="organizationId.set('org-b')">Changer d'organisation</button>
      <app-audit-log [organizationId]="organizationId()" />
    `,
    moduleMetadata: { imports: [AuditLog] },
  }),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('status')).toHaveTextContent('Chargement…')

    await userEvent.click(canvas.getByRole('button', { name: "Changer d'organisation" }))
    expect(await canvas.findByText('Utilisateur invité')).toBeInTheDocument()

    slowOrgA.next(page(ENTRIES))
    slowOrgA.complete()
    await new Promise((resolve) => setTimeout(resolve, 50))
    await waitFor(() => expect(canvas.getByText('Utilisateur invité')).toBeInTheDocument())
    expect(canvas.queryByText('PermissionGranted')).not.toBeInTheDocument()
  },
}
