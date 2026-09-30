import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { ActivatedRoute, Router, convertToParamMap } from '@angular/router'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { of } from 'rxjs'
import { OrganizationsPage } from './organizations-page'
import { MeService } from '../../shell/application/me.service'
import { OrganizationsService } from '../application/organizations.service'
import { OrganizationMembersService } from '../application/organization-members.service'
import { BrandingService } from '../application/branding.service'
import { AdminApiTokensService } from '../application/admin-api-tokens.service'
import { AuditService } from '../application/audit.service'
import { AdminMetricsService } from '../application/metrics.service'
import { SystemSettingsService } from '../application/system-settings.service'
import { SmtpSettingsService } from '../application/smtp-settings.service'
import { UsersService } from '../../users/application/users.service'
import { RepositoriesService } from '../../repositories/application/repositories.service'
import type { OrganizationSummary } from '../domain/organization.entity'

const ACME_ORG: OrganizationSummary = {
  id: 'org-acme',
  slug: 'acme',
  display_name: 'Acme Corp',
  is_public: false,
}

function withRoute(id: string | null) {
  const params = id ? { id } : {}
  return {
    provide: ActivatedRoute,
    useValue: { paramMap: of(convertToParamMap(params)) },
  }
}

// A real Router (via provideRouter) tries to match the Storybook iframe's own URL against the
// (empty) route table and errors — this page's children only ever call .navigate(), so a plain
// stub sidesteps that entirely.
const routerStub = { provide: Router, useValue: { navigate: () => Promise.resolve(true) } }

const meta: Meta<OrganizationsPage> = {
  title: 'Admin/OrganizationsPage',
  component: OrganizationsPage,
  decorators: [
    moduleMetadata({
      providers: [
        routerStub,
        withRoute(null),
        { provide: MeService, useValue: { isSuperAdmin: () => true } },
        { provide: OrganizationsService, useValue: { list: () => of([ACME_ORG]) } },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<OrganizationsPage>

/** A super-admin with no organization selected sees only the list. */
export const OrganizationsListOnly: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('Acme Corp')).toBeInTheDocument())
    expect(canvas.queryByRole('tab', { name: 'Aperçu' })).not.toBeInTheDocument()
  },
}

const AUDIT_ENTRY = {
  aggregate_type: 'Repository',
  aggregate_id: 'repo-1',
  event_type: 'RepositoryCreated',
  payload: {},
  occurred_at: '2026-03-01T09:30:00Z',
  actor_id: 'u-alice',
}
const SECURITY_ENTRY = {
  aggregate_type: 'Security',
  aggregate_id: 'u-alice',
  event_type: 'LoginFailed',
  payload: {},
  occurred_at: '2026-03-02T10:00:00Z',
  actor_id: 'u-alice',
}

/** Fakes for everything the ten tabs of the detail panel inject, scoped to one organization. */
function fakeDetailServices() {
  const audit = {
    query: fn((filter: { aggregate_type?: string }) =>
      of({
        entries: filter.aggregate_type === 'Security' ? [SECURITY_ENTRY] : [AUDIT_ENTRY],
        next_cursor: null,
      }),
    ),
    blockedAccounts: fn(() => of([{ username: 'mallory', remaining_seconds: 600 }])),
  }
  const metrics = {
    stats: fn(() => of({ total_users: 12, total_repositories: 4, total_active_permissions: 30 })),
    usage: fn(() =>
      of([{ repository_id: 'repo-1', name: 'acme-npm', used_bytes: 2_000_000, quota_bytes: null }]),
    ),
  }
  const apiTokens = {
    list: fn(() =>
      of([
        {
          id: 'tok-1',
          user_id: 'u-alice',
          username: 'alice',
          label: 'ci-bot',
          created_at: '2026-03-01T09:30:00Z',
          last_used_at: null,
          revoked_at: null,
        },
      ]),
    ),
    revoke: fn(() => of(undefined)),
  }
  const systemSettings = {
    get: fn(() =>
      of({
        max_login_attempts: 5,
        login_attempt_window_seconds: 900,
        session_ttl_hours: 24,
        registration_enabled: true,
        seo_indexing_enabled: false,
        seo_indexing_blocked: false,
        public_page_enabled: true,
      }),
    ),
    update: fn(() => of(undefined)),
  }
  const smtp = {
    get: fn(() =>
      of({
        host: 'smtp.acme.test',
        port: 587,
        username: 'mailer',
        from_name: 'Acme Corp',
        from_address: 'noreply@acme.test',
        security: 'start_tls' as const,
        password_set: true,
      }),
    ),
    update: fn(() => of(undefined)),
    sendTestEmail: fn(() => of(undefined)),
  }
  const organizations = {
    list: fn(() => of([ACME_ORG])),
    get: fn(() => of(ACME_ORG)),
    getIdentityProvider: fn(() => of({ type: null as null })),
  }
  const members = {
    list: fn(() =>
      of([
        {
          id: 'u-alice',
          username: 'alice',
          email: 'alice@acme.test',
          is_organization_admin: true,
          invitation_pending: false,
        },
      ]),
    ),
  }
  const branding = {
    getLogo: fn(() => of(new Blob(['logo'], { type: 'image/png' }))),
    getFavicon: fn(() => of(new Blob(['favicon'], { type: 'image/png' }))),
  }
  const decorator = moduleMetadata({
    providers: [
      withRoute('org-acme'),
      { provide: OrganizationsService, useValue: organizations },
      { provide: OrganizationMembersService, useValue: members },
      { provide: BrandingService, useValue: branding },
      { provide: AdminApiTokensService, useValue: apiTokens },
      { provide: AuditService, useValue: audit },
      { provide: AdminMetricsService, useValue: metrics },
      { provide: SystemSettingsService, useValue: systemSettings },
      { provide: SmtpSettingsService, useValue: smtp },
      { provide: UsersService, useValue: { list: () => of([]) } },
      { provide: RepositoriesService, useValue: { list: () => of([]) } },
    ],
  })
  return { decorator, audit, metrics, apiTokens, systemSettings, smtp, organizations }
}

function asOrganizationAdmin() {
  return moduleMetadata({
    providers: [{ provide: MeService, useValue: { isSuperAdmin: () => false } }],
  })
}

async function openTab(canvas: ReturnType<typeof within>, name: string) {
  await userEvent.click(await canvas.findByRole('tab', { name }))
  return canvas.findByRole('tabpanel', { name })
}

const superAdminDetail = fakeDetailServices()
/** A super-admin on /admin/organizations/:id sees the list next to the selected organization's tabs. */
export const SuperAdminWithOrganizationSelected: Story = {
  decorators: [superAdminDetail.decorator],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const tabs = [
      'Aperçu',
      'Marque',
      'Jetons API',
      'Historique',
      'Sécurité',
      'Métriques',
      'Paramètres',
      'Serveur mail',
    ]
    for (const name of tabs) {
      expect(await canvas.findByRole('tab', { name })).toBeInTheDocument()
    }
    expect(canvas.getByRole('table', { name: 'Organisations' })).toBeInTheDocument()
    expect(canvas.getByRole('tab', { name: 'Aperçu' })).toHaveAttribute('aria-selected', 'true')

    const overview = await canvas.findByRole('tabpanel', { name: 'Aperçu' })
    expect(await within(overview).findByRole('heading', { name: 'Acme Corp' })).toBeInTheDocument()
    expect(within(overview).getByText('Sous-domaine : acme')).toBeInTheDocument()
    expect(superAdminDetail.organizations.get).toHaveBeenCalledWith('org-acme')
  },
}

const orgAdminDetail = fakeDetailServices()
/** A plain organization admin never sees the organizations list, only their own organization's tabs. */
export const OrganizationAdminSeesOnlyTheirOrganization: Story = {
  decorators: [orgAdminDetail.decorator, asOrganizationAdmin()],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const overview = await canvas.findByRole('tabpanel', { name: 'Aperçu' })
    expect(await within(overview).findByRole('heading', { name: 'Acme Corp' })).toBeInTheDocument()
    expect(canvas.queryByRole('table', { name: 'Organisations' })).not.toBeInTheDocument()
    expect(canvas.queryByRole('button', { name: 'Nouvelle organisation' })).not.toBeInTheDocument()
    expect(orgAdminDetail.organizations.list).not.toHaveBeenCalled()
  },
}

export const BrandingTab: Story = {
  decorators: [fakeDetailServices().decorator],
  play: async ({ canvasElement }) => {
    const panel = await openTab(within(canvasElement), 'Marque')
    expect(await within(panel).findByAltText('Logo actuel')).toBeInTheDocument()
    expect(within(panel).getByAltText('Favicon actuel')).toBeInTheDocument()
  },
}

const apiTokensDetail = fakeDetailServices()
export const ApiTokensTab: Story = {
  decorators: [apiTokensDetail.decorator],
  play: async ({ canvasElement }) => {
    const panel = await openTab(within(canvasElement), 'Jetons API')
    expect(await within(panel).findByText('ci-bot')).toBeInTheDocument()
    expect(apiTokensDetail.apiTokens.list).toHaveBeenCalledWith('org-acme')
  },
}

const auditDetail = fakeDetailServices()
export const AuditLogTab: Story = {
  decorators: [auditDetail.decorator],
  play: async ({ canvasElement }) => {
    const panel = await openTab(within(canvasElement), 'Historique')
    expect(await within(panel).findByRole('table', { name: "Journal d'audit" })).toBeInTheDocument()
    expect(within(panel).getByText('RepositoryCreated')).toBeInTheDocument()
    expect(within(panel).queryByText('Échec de connexion')).not.toBeInTheDocument()
    expect(auditDetail.audit.query).toHaveBeenCalledWith({
      exclude_aggregate_type: 'Security',
      organization_id: 'org-acme',
    })
  },
}

const securityDetail = fakeDetailServices()
/** Scoped to an organization, the security log leaves out the instance-wide blocked-accounts panel. */
export const SecurityLogTab: Story = {
  decorators: [securityDetail.decorator],
  play: async ({ canvasElement }) => {
    const panel = await openTab(within(canvasElement), 'Sécurité')
    expect(
      await within(panel).findByRole('table', { name: 'Journal de sécurité' }),
    ).toBeInTheDocument()
    // Once in the summary card and once as a row of the log.
    expect(within(panel).getAllByText('Échec de connexion')).toHaveLength(2)
    expect(within(panel).queryByText('Comptes actuellement bloqués')).not.toBeInTheDocument()
    expect(securityDetail.audit.query).toHaveBeenCalledWith({
      aggregate_type: 'Security',
      organization_id: 'org-acme',
    })
    expect(securityDetail.audit.blockedAccounts).not.toHaveBeenCalled()
  },
}

const metricsDetail = fakeDetailServices()
export const MetricsTab: Story = {
  decorators: [metricsDetail.decorator],
  play: async ({ canvasElement }) => {
    const panel = await openTab(within(canvasElement), 'Métriques')
    expect(await within(panel).findByRole('heading', { name: 'Utilisateurs' })).toBeInTheDocument()
    expect(within(panel).getByText('12')).toBeInTheDocument()
    expect(within(panel).getByRole('table', { name: 'Utilisation par dépôt' })).toBeInTheDocument()
    expect(metricsDetail.metrics.stats).toHaveBeenCalledWith('org-acme')
    expect(metricsDetail.metrics.usage).toHaveBeenCalledWith('org-acme')
  },
}

const settingsDetail = fakeDetailServices()
export const SystemSettingsTab: Story = {
  decorators: [settingsDetail.decorator],
  play: async ({ canvasElement }) => {
    const panel = await openTab(within(canvasElement), 'Paramètres')
    await waitFor(() =>
      expect(within(panel).getByLabelText('Tentatives de connexion max')).toHaveValue('5'),
    )
    expect(settingsDetail.systemSettings.get).toHaveBeenCalledWith('org-acme')
  },
}

const smtpDetail = fakeDetailServices()
export const SmtpTab: Story = {
  decorators: [smtpDetail.decorator],
  play: async ({ canvasElement }) => {
    const panel = await openTab(within(canvasElement), 'Serveur mail')
    await waitFor(() =>
      expect(within(panel).getByLabelText('Hôte SMTP')).toHaveValue('smtp.acme.test'),
    )
    expect(smtpDetail.smtp.get).toHaveBeenCalledWith('org-acme')
  },
}
