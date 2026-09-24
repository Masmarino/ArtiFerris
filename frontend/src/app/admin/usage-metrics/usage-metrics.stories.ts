import { signal } from '@angular/core'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { of, throwError } from 'rxjs'
import { UsageMetrics } from './usage-metrics'
import { AdminMetricsService } from '../application/metrics.service'
import type { RepositoryUsage } from '../domain/metrics.entity'

const MB = 1024 * 1024
const GB = 1024 * MB

const USAGES: RepositoryUsage[] = [
  { repository_id: 'r1', name: 'npm-small', used_bytes: 100 * MB, quota_bytes: GB },
  { repository_id: 'r2', name: 'web-assets', used_bytes: 800 * MB, quota_bytes: GB },
  { repository_id: 'r3', name: 'docker-images', used_bytes: 950 * MB, quota_bytes: GB },
  { repository_id: 'r4', name: 'unlimited-mirror', used_bytes: 2 * GB, quota_bytes: null },
]

function fakeMetrics(overrides: Partial<AdminMetricsService> = {}): Partial<AdminMetricsService> {
  return { usage: () => of(USAGES), ...overrides }
}

const meta: Meta<UsageMetrics> = {
  title: 'Admin/UsageMetrics',
  component: UsageMetrics,
  decorators: [
    moduleMetadata({ providers: [{ provide: AdminMetricsService, useValue: fakeMetrics() }] }),
  ],
}
export default meta

type Story = StoryObj<UsageMetrics>

function gaugeTier(gauge: HTMLElement): string | null {
  return gauge.querySelector('.gbt-gauge-bar__fill')?.getAttribute('data-tier') ?? null
}

/** Chart sorted by usage with formatted sizes, quota gauges only for capped repositories, and the full table. */
export const Populated: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const chart = await canvas.findByRole('table', { name: 'Dépôts les plus volumineux' })
    const labels = within(chart)
      .getAllByRole('rowheader')
      .map((el) => el.textContent?.trim())
    expect(labels).toEqual(['unlimited-mirror', 'docker-images', 'web-assets', 'npm-small'])
    expect(within(chart).getByText('2.0 Go')).toBeInTheDocument()

    const table = canvas.getByRole('table', { name: 'Utilisation par dépôt' })
    expect(within(table).getAllByRole('row')).toHaveLength(USAGES.length + 1)
    expect(within(table).getByText(String(800 * MB))).toBeInTheDocument()
  },
}

/** Below 70% no tier, 70% and up warns, 90% and up is critical; unlimited repositories get no gauge. */
export const QuotaGauges: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Quotas de stockage')).toBeInTheDocument()
    expect(canvas.getAllByRole('progressbar')).toHaveLength(3)

    const small = canvas.getByRole('progressbar', { name: 'npm-small' })
    const warn = canvas.getByRole('progressbar', { name: 'web-assets' })
    const critical = canvas.getByRole('progressbar', { name: 'docker-images' })
    expect(small).toHaveAttribute('aria-valuetext', '100.0 Mo / 1.0 Go')
    expect(gaugeTier(small)).toBeNull()
    expect(gaugeTier(warn)).toBe('warning')
    expect(gaugeTier(critical)).toBe('critical')
    expect(canvas.getByText('Critique')).toBeInTheDocument()
    expect(canvas.getByText('Avertissement')).toBeInTheDocument()
    expect(canvas.queryByRole('progressbar', { name: 'unlimited-mirror' })).not.toBeInTheDocument()
  },
}

export const NoQuotas: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AdminMetricsService,
          useValue: fakeMetrics({
            usage: () => of(USAGES.map((u) => ({ ...u, quota_bytes: null }))),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await canvas.findByRole('table', { name: 'Utilisation par dépôt' })
    expect(canvas.queryByText('Quotas de stockage')).not.toBeInTheDocument()
    expect(canvas.queryByRole('progressbar')).not.toBeInTheDocument()
  },
}

export const Empty: Story = {
  decorators: [
    moduleMetadata({
      providers: [{ provide: AdminMetricsService, useValue: fakeMetrics({ usage: () => of([]) }) }],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Aucun dépôt.')).toBeInTheDocument()
    expect(canvas.getByText("Aucun détail d'utilisation disponible.")).toBeInTheDocument()
    expect(canvas.queryByText('Quotas de stockage')).not.toBeInTheDocument()
  },
}

let failedOnce = false

/** A failed request shows an error with a retry; the retry loads the data. */
export const LoadFailed: Story = {
  beforeEach: () => {
    failedOnce = false
  },
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AdminMetricsService,
          useValue: fakeMetrics({
            usage: () => {
              if (!failedOnce) {
                failedOnce = true
                return throwError(() => new Error('network error'))
              }
              return of(USAGES)
            },
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('alert')).toHaveTextContent(
      "Échec du chargement de l'utilisation",
    )

    await userEvent.click(canvas.getByRole('button', { name: 'Réessayer' }))

    expect(
      await canvas.findByRole('table', { name: 'Dépôts les plus volumineux' }),
    ).toBeInTheDocument()
    expect(canvas.queryByRole('alert')).not.toBeInTheDocument()
  },
}

const MANY: RepositoryUsage[] = Array.from({ length: 17 }, (_, i) => ({
  repository_id: `r${i}`,
  name: `repo-${i}`,
  used_bytes: (100 - i) * MB,
  quota_bytes: null,
}))

/** Past 15 repositories the chart folds the tail into one "Autres" bar; the table still lists all of them. */
export const MoreThanFifteenRepositories: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        { provide: AdminMetricsService, useValue: fakeMetrics({ usage: () => of(MANY) }) },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const chart = await canvas.findByRole('table', { name: 'Dépôts les plus volumineux' })
    expect(within(chart).getAllByRole('rowheader')).toHaveLength(16)
    const folded = within(chart).getByText('Autres (2)')
    // The two smallest: 85 Mo + 84 Mo.
    expect(folded.closest('tr')).toHaveTextContent('169.0 Mo')

    const table = canvas.getByRole('table', { name: 'Utilisation par dépôt' })
    expect(within(table).getAllByRole('row')).toHaveLength(MANY.length + 1)
  },
}

const scopedUsage = fn<AdminMetricsService['usage']>(() => of(USAGES))

export const ScopedToOrganization: Story = {
  args: { organizationId: 'org-acme' },
  decorators: [
    moduleMetadata({
      providers: [{ provide: AdminMetricsService, useValue: { usage: scopedUsage } }],
    }),
  ],
  play: async ({ canvasElement }) => {
    await within(canvasElement).findByRole('table', { name: 'Utilisation par dépôt' })
    expect(scopedUsage).toHaveBeenLastCalledWith('org-acme')
  },
}

/** The component is reused when a super-admin switches organization: it must re-fetch and replace the data. */
export const SwitchOrganization: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AdminMetricsService,
          useValue: fakeMetrics({
            usage: (organizationId) =>
              of(
                organizationId === 'org-b'
                  ? [
                      {
                        repository_id: 'b1',
                        name: 'org-b-repo',
                        used_bytes: 5 * MB,
                        quota_bytes: null,
                      },
                    ]
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
      <app-usage-metrics [organizationId]="organizationId()" />
    `,
    moduleMetadata: { imports: [UsageMetrics] },
  }),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const table = await canvas.findByRole('table', { name: 'Utilisation par dépôt' })
    expect(within(table).getByText('docker-images')).toBeInTheDocument()

    await userEvent.click(canvas.getByRole('button', { name: "Changer d'organisation" }))
    await waitFor(() => expect(within(table).getByText('org-b-repo')).toBeInTheDocument())
    expect(within(table).queryByText('docker-images')).not.toBeInTheDocument()
    expect(canvas.queryByText('Quotas de stockage')).not.toBeInTheDocument()
  },
}
