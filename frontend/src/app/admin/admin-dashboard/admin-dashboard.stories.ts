import { signal } from '@angular/core'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { NEVER, of, throwError } from 'rxjs'
import { AdminDashboard } from './admin-dashboard'
import { AdminMetricsService } from '../application/metrics.service'
import { AuditService } from '../application/audit.service'
import { RepositoriesService } from '../../repositories/application/repositories.service'
import type { AdminStats, MetricsSnapshot } from '../domain/metrics.entity'
import type { AuditEntry } from '../domain/audit.entity'
import type { RepositorySummary } from '../../repositories/domain/repository.entity'
import { PageTitleService } from '../../shell/page-title.service'

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
    is_public: false,
    my_role: 'admin',
    organization_id: 'org-public',
    owner_name: 'Public',
    owner_is_personal: false,
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
    is_public: false,
    my_role: 'admin',
    organization_id: 'org-acme',
    owner_name: 'Acme Corp',
    owner_is_personal: false,
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
    is_public: false,
    my_role: 'admin',
    organization_id: 'org-acme',
    owner_name: 'Acme Corp',
    owner_is_personal: false,
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
    event_type: 'UserActivated',
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
  return { query: () => of({ entries: ACTIVITY, next_cursor: null }), ...overrides }
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
        // The shell sets the title from the route.
        { provide: PageTitleService, useValue: { title: signal('Tableau de bord') } },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<AdminDashboard>

const tileValue = (canvasElement: HTMLElement, key: string) =>
  canvasElement.querySelector(`[data-key="${key}"]`)?.textContent ?? ''

export const Default: Story = {
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(tileValue(canvasElement, 'users')).toContain('12'))
    expect(tileValue(canvasElement, 'repositories')).toContain('5')
    expect(tileValue(canvasElement, 'permissions')).toContain('18')
    expect(tileValue(canvasElement, 'storage')).toContain('dernier relevé')
  },
}

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
        {
          provide: AuditService,
          useValue: fakeAudit({ query: () => of({ entries: [], next_cursor: null }) }),
        },
        { provide: RepositoriesService, useValue: fakeRepositories({ list: () => of([]) }) },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Aucune activité récente')).toBeInTheDocument()
    expect(canvas.getByText('Aucun dépôt à répartir par format.')).toBeInTheDocument()
    expect(canvas.getAllByText('Pas encore assez de mesures')).toHaveLength(2)
    expect(tileValue(canvasElement, 'storage')).toContain('aucun relevé')
  },
}

export const LoadFailed: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AdminMetricsService,
          useValue: fakeMetrics({ stats: () => throwError(() => new Error('boom')) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(
      await canvas.findByText("Une partie du tableau de bord n'a pas pu être chargée."),
    ).toBeInTheDocument()
    expect(canvas.getAllByRole('button', { name: 'Réessayer' })[0]).toBeEnabled()
  },
}

export const Loading: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AdminMetricsService,
          useValue: fakeMetrics({ stats: () => NEVER, history: () => NEVER }),
        },
        { provide: AuditService, useValue: fakeAudit({ query: () => NEVER }) },
        { provide: RepositoriesService, useValue: fakeRepositories({ list: () => NEVER }) },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText("Chargement de l'historique…")).toBeInTheDocument()
  },
}

const MANY_EVENTS: AuditEntry[] = Array.from({ length: 12 }, (_, i) => ({
  aggregate_type: 'Package',
  aggregate_id: `p${i}`,
  event_type: `Published${i}`,
  payload: {},
  occurred_at: new Date().toISOString(),
  actor_id: 'u1',
}))

export const RecentActivityIsCappedAtTen: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuditService,
          useValue: fakeAudit({ query: () => of({ entries: MANY_EVENTS, next_cursor: null }) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() =>
      expect(canvasElement.querySelectorAll('.admin-dashboard__recent li')).toHaveLength(10),
    )
  },
}

export const ActivityChartIsPartial: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AuditService,
          useValue: fakeAudit({ query: () => of({ entries: MANY_EVENTS, next_cursor: 'older' }) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(
      await canvas.findByText(/Basé sur les 12 événements les plus récents/),
    ).toBeInTheDocument()
  },
}

const history = fn<AdminMetricsService['history']>(() => of(HISTORY))

export const ChangingThePeriod: Story = {
  beforeEach: () => {
    history.mockClear()
  },
  decorators: [
    moduleMetadata({
      providers: [{ provide: AdminMetricsService, useValue: fakeMetrics({ history }) }],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(history).toHaveBeenCalledWith(30))
    await userEvent.click(await canvas.findByRole('radio', { name: '3 jours' }))
    await waitFor(() => expect(history).toHaveBeenLastCalledWith(3))
    expect(history).toHaveBeenCalledTimes(2)
  },
}
