import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { ActivatedRoute, Router, convertToParamMap } from '@angular/router'
import { expect, waitFor, within } from 'storybook/test'
import { of } from 'rxjs'
import { OrganizationsPage } from './organizations-page'
import { MeService } from '../../shell/application/me.service'
import { OrganizationsService } from '../application/organizations.service'
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

// Only the no-selection view is covered here. Selecting an organization mounts its tabbed
// detail panel, which is ten real child components (OrganizationDetail, OrganizationMembers,
// BrandingSettingsAdmin, ApiTokensAdmin, AuditLog, SecurityLog, OrganizationMetricsPage,
// UsageMetrics, SystemSettingsAdmin, SmtpSettingsAdmin), each scheduling its own effect() on
// construction. Reproduced in isolation: any nine of the ten mounted together render and test
// fine, in any combination, regardless of whether their own data call succeeds or errors — but
// all ten mounting together reliably hangs the real browser this addon's interaction tests run
// in (confirmed via repeated bisection, not a one-off flake or a system-load artifact). There is
// no way to keep every tab mounted while dodging this from a story file, since preventing any
// one component from constructing (e.g. via a missing provider) is the only thing that avoids
// it. Until this is root-caused (most likely a zoneless effect-scheduling issue in this Angular
// version, triggered only with real layout in a real browser — the plain-TestBed spec for this
// component already covers the with-organization-selected case and passes) or the addon-vitest
// browser runner is more resilient to it, don't add a story that sets a route id here.

/** A super-admin with no organization selected sees only the list. */
export const OrganizationsListOnly: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('Acme Corp')).toBeInTheDocument())
    expect(canvas.queryByRole('tab', { name: 'Aperçu' })).not.toBeInTheDocument()
  },
}
