import { signal } from '@angular/core'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { NEVER, of } from 'rxjs'
import { OrganizationMetricsPage } from './organization-metrics-page'
import { AdminMetricsService } from '../application/metrics.service'
import type { AdminStats, RepositoryUsage } from '../domain/metrics.entity'

const MB = 1024 * 1024

const STATS: AdminStats = {
  total_users: 12,
  total_repositories: 5,
  total_active_permissions: 18,
}

const USAGES: RepositoryUsage[] = [
  { repository_id: 'r1', name: 'acme-npm', used_bytes: 300 * MB, quota_bytes: 1024 * MB },
  { repository_id: 'r2', name: 'acme-docker', used_bytes: 50 * MB, quota_bytes: null },
]

function fakeMetrics(overrides: Partial<AdminMetricsService> = {}): Partial<AdminMetricsService> {
  return { stats: () => of(STATS), usage: () => of(USAGES), ...overrides }
}

const meta: Meta<OrganizationMetricsPage> = {
  title: 'Admin/OrganizationMetricsPage',
  component: OrganizationMetricsPage,
  decorators: [
    moduleMetadata({ providers: [{ provide: AdminMetricsService, useValue: fakeMetrics() }] }),
  ],
}
export default meta

type Story = StoryObj<OrganizationMetricsPage>

export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('12')).toBeInTheDocument())
    expect(canvas.getByText('Utilisateurs')).toBeInTheDocument()
    expect(canvas.getByText('5')).toBeInTheDocument()
    expect(canvas.getByText('Dépôts')).toBeInTheDocument()
    expect(canvas.getByText('18')).toBeInTheDocument()
    expect(canvas.getByText("Droits d'accès")).toBeInTheDocument()

    expect(canvas.getByText('Espace utilisé par dépôt')).toBeInTheDocument()
    expect(canvas.getByRole('progressbar', { name: 'acme-npm' })).toBeInTheDocument()
  },
}

export const EmptyOrganization: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AdminMetricsService,
          useValue: fakeMetrics({
            stats: () => of({ total_users: 0, total_repositories: 0, total_active_permissions: 0 }),
            usage: () => of([]),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Aucun dépôt.')).toBeInTheDocument()
    expect(canvas.getAllByText('0')).toHaveLength(3)
    expect(canvas.queryByText('Quotas de stockage')).not.toBeInTheDocument()
  },
}

export const StatsPending: Story = {
  decorators: [
    moduleMetadata({
      providers: [{ provide: AdminMetricsService, useValue: fakeMetrics({ stats: () => NEVER }) }],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Espace utilisé par dépôt')).toBeInTheDocument()
    expect(canvas.queryByText('Utilisateurs')).not.toBeInTheDocument()
    expect(canvas.queryByText("Droits d'accès")).not.toBeInTheDocument()
  },
}

const scopedStats = fn<AdminMetricsService['stats']>(() => of(STATS))
const scopedUsage = fn<AdminMetricsService['usage']>(() => of(USAGES))

export const ScopedToOrganization: Story = {
  args: { organizationId: 'org-acme' },
  decorators: [
    moduleMetadata({
      providers: [
        { provide: AdminMetricsService, useValue: { stats: scopedStats, usage: scopedUsage } },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await within(canvasElement).findByText('Utilisateurs')
    expect(scopedStats).toHaveBeenLastCalledWith('org-acme')
    expect(scopedUsage).toHaveBeenLastCalledWith('org-acme')
  },
}

export const SwitchOrganization: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AdminMetricsService,
          useValue: fakeMetrics({
            stats: (organizationId) =>
              of(
                organizationId === 'org-b'
                  ? { total_users: 77, total_repositories: 8, total_active_permissions: 9 }
                  : STATS,
              ),
            usage: (organizationId) =>
              of(
                organizationId === 'org-b'
                  ? [{ repository_id: 'b1', name: 'org-b-repo', used_bytes: MB, quota_bytes: null }]
                  : USAGES,
              ),
          }),
        },
      ],
    }),
  ],
  render: () => ({
    props: { organizationId: signal('org-a') },
    template: `
      <button type="button" (click)="organizationId.set('org-b')">Changer d'organisation</button>
      <app-organization-metrics-page [organizationId]="organizationId()" />
    `,
    moduleMetadata: { imports: [OrganizationMetricsPage] },
  }),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('12')).toBeInTheDocument())

    await userEvent.click(canvas.getByRole('button', { name: "Changer d'organisation" }))
    await waitFor(() => expect(canvas.getByText('77')).toBeInTheDocument())
    expect(canvas.queryByText('12')).not.toBeInTheDocument()
    expect(canvas.getAllByText('org-b-repo').length).toBeGreaterThan(0)
    expect(canvas.queryByText('acme-npm')).not.toBeInTheDocument()
  },
}
