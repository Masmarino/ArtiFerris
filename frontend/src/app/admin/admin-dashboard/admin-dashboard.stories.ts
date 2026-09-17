import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { of } from 'rxjs'
import { AdminDashboard } from './admin-dashboard'
import { AdminMetricsService } from '../application/metrics.service'
import { AuditService } from '../application/audit.service'
import { RepositoriesService } from '../../repositories/application/repositories.service'
import type { AdminStats, MetricsSnapshot } from '../domain/metrics.entity'
import type { AuditEntry } from '../domain/audit.entity'
import type { RepositorySummary } from '../../repositories/domain/repository.entity'

const STATS: AdminStats = {
  total_users: 12,
  total_repositories: 5,
  total_active_permissions: 18,
}

const REPOSITORIES: RepositorySummary[] = [
  {
    id: 'r1',
    name: 'artiferris-web',
    format: 'npm',
    repo_type: 'hosted',
    remote_url: null,
    remote_credentials_set: false,
    group_members: [],
    quota_bytes: null,
    retention_keep_last_n: null,
    my_role: 'admin',
    organization_id: 'org-public',
  },
  {
    id: 'r2',
    name: 'acme-docker',
    format: 'docker',
    repo_type: 'hosted',
    remote_url: null,
    remote_credentials_set: false,
    group_members: [],
    quota_bytes: null,
    retention_keep_last_n: null,
    my_role: 'admin',
    organization_id: 'org-acme',
  },
  {
    id: 'r3',
    name: 'acme-npm',
    format: 'npm',
    repo_type: 'hosted',
    remote_url: null,
    remote_credentials_set: false,
    group_members: [],
    quota_bytes: null,
    retention_keep_last_n: null,
    my_role: 'admin',
    organization_id: 'org-acme',
  },
]

const ACTIVITY: AuditEntry[] = [
  {
    aggregate_type: 'PackageRepository',
    aggregate_id: 'r1',
    event_type: 'Created',
    payload: {},
    occurred_at: new Date().toISOString(),
    actor_id: 'u1',
  },
  {
    aggregate_type: 'User',
    aggregate_id: 'u2',
    event_type: 'Registered',
    payload: {},
    occurred_at: new Date().toISOString(),
    actor_id: null,
  },
]

const HISTORY: MetricsSnapshot[] = [
  {
    recorded_at: '2026-09-10T00:00:00Z',
    total_users: 8,
    total_repositories: 3,
    total_storage_bytes: 1_000_000,
  },
  {
    recorded_at: '2026-09-17T00:00:00Z',
    total_users: 12,
    total_repositories: 5,
    total_storage_bytes: 2_000_000,
  },
]

function fakeMetrics(overrides: Partial<AdminMetricsService> = {}): Partial<AdminMetricsService> {
  return { stats: () => of(STATS), history: () => of(HISTORY), ...overrides }
}
function fakeAudit(overrides: Partial<AuditService> = {}): Partial<AuditService> {
  return { query: () => of(ACTIVITY), ...overrides }
}
function fakeRepositories(
  overrides: Partial<RepositoriesService> = {},
): Partial<RepositoriesService> {
  return { list: () => of(REPOSITORIES), ...overrides }
}

const meta: Meta<AdminDashboard> = {
  title: 'Admin/AdminDashboard',
  component: AdminDashboard,
  decorators: [
    moduleMetadata({
      providers: [
        { provide: AdminMetricsService, useValue: fakeMetrics() },
        { provide: AuditService, useValue: fakeAudit() },
        { provide: RepositoriesService, useValue: fakeRepositories() },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<AdminDashboard>

// The same total can also appear in a chart's accessible data table or legend —
// scope the assertion to the stat card itself, identified by its heading.
function statCardValue(canvas: ReturnType<typeof within>, heading: string): string | null {
  const card = canvas.getByRole('heading', { name: heading }).closest('.gbt-card')
  return card ? (within(card as HTMLElement).getByText(/^\d+$/).textContent ?? null) : null
}

/** Totals, both charts and the recent-activity list all populated. */
export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(statCardValue(canvas, 'Utilisateurs')).toBe('12'))
    expect(statCardValue(canvas, 'Dépôts')).toBe('5')
    expect(statCardValue(canvas, 'Permissions actives')).toBe('18')
  },
}

/** A brand new instance: no activity yet, no repositories, no history. */
export const Empty: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AdminMetricsService,
          useValue: fakeMetrics({
            stats: () => of({ total_users: 0, total_repositories: 0, total_active_permissions: 0 }),
            history: () => of([]),
          }),
        },
        { provide: AuditService, useValue: fakeAudit({ query: () => of([]) }) },
        { provide: RepositoriesService, useValue: fakeRepositories({ list: () => of([]) }) },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('Aucune activité récente.')).toBeInTheDocument())
  },
}

/** Changing the storage evolution window re-fetches only that chart's history. */
export const ChangingTheStorageEvolutionWindow: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(statCardValue(canvas, 'Utilisateurs')).toBe('12'))
    const comboboxes = canvas.getAllByRole('combobox')
    await userEvent.click(comboboxes[0])
    await userEvent.click(await canvas.findByRole('option', { name: '1 mois' }))
  },
}
