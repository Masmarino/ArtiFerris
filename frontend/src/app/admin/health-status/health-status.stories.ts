import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, waitFor, within } from 'storybook/test'
import { of, throwError } from 'rxjs'
import { HealthStatusPage } from './health-status'
import { AdminMetricsService } from '../application/metrics.service'
import type { HealthStatus } from '../domain/metrics.entity'

const HEALTHY: HealthStatus = {
  database: {
    status: 'up',
    detail: null,
    response_time_ms: 4,
    active_connections: 3,
    max_connections: 10,
    server_version: '18.0',
  },
  storage: {
    status: 'up',
    detail: null,
    used_bytes: 100_000_000,
    free_bytes: 900_000_000,
    total_bytes: 1_000_000_000,
  },
  uptime_seconds: 90_000,
}

const DEGRADED: HealthStatus = {
  ...HEALTHY,
  database: { ...HEALTHY.database, status: 'down', detail: 'connection refused' },
  storage: {
    status: 'down',
    detail: 'disk full',
    used_bytes: 950_000_000,
    free_bytes: 50_000_000,
    total_bytes: 1_000_000_000,
  },
}

function fakeMetrics(overrides: Partial<AdminMetricsService> = {}): Partial<AdminMetricsService> {
  return { health: () => of(HEALTHY), ...overrides }
}

const meta: Meta<HealthStatusPage> = {
  title: 'Admin/HealthStatusPage',
  component: HealthStatusPage,
  decorators: [
    moduleMetadata({ providers: [{ provide: AdminMetricsService, useValue: fakeMetrics() }] }),
  ],
}
export default meta

type Story = StoryObj<HealthStatusPage>

/** A healthy instance: database and storage both up, comfortable gauges. */
export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('18.0')).toBeInTheDocument())
    expect(canvas.getByText('3 / 10')).toBeInTheDocument()
    expect(canvas.getByText('1 j 1 h 0 min')).toBeInTheDocument()
  },
}

/** Database down and storage nearly full — both statuses and details surface. */
export const Degraded: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        { provide: AdminMetricsService, useValue: fakeMetrics({ health: () => of(DEGRADED) }) },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText(/connection refused/)).toBeInTheDocument())
    expect(canvas.getByText(/disk full/)).toBeInTheDocument()
    const gauges: HTMLElement[] = Array.from(
      canvasElement.querySelectorAll('gbt-gauge-bar .gbt-gauge-bar__fill'),
    )
    expect(gauges[1].getAttribute('data-tier')).toBe('critical')
  },
}

export const LoadFailed: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: AdminMetricsService,
          useValue: fakeMetrics({ health: () => throwError(() => new Error('down')) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() =>
      expect(within(canvasElement).getByRole('alert')).toHaveTextContent(
        "Échec du chargement de l'état du système.",
      ),
    )
  },
}
