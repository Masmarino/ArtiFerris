import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { ActivatedRoute, Router, convertToParamMap } from '@angular/router'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { of, throwError } from 'rxjs'
import { PackageDetailPage } from './package-detail-page'
import { RepositoriesService } from '../application/repositories.service'
import type {
  DockerImageDetails,
  DockerImageScanResult,
  NpmAdvisory,
  NpmDependencyAuditResult,
  NpmPackageDetails,
  RepositorySummary,
} from '../domain/repository.entity'

const NPM_DETAILS: NpmPackageDetails = {
  name: '@acme/ui',
  versions: [
    {
      version: '1.2.0',
      published_at: '2026-09-01T00:00:00Z',
      size_bytes: 42_000,
      deprecated: false,
      deprecated_message: null,
      shasum: 'abc',
    },
    {
      version: '1.1.0',
      published_at: '2026-08-01T00:00:00Z',
      size_bytes: 40_000,
      deprecated: true,
      deprecated_message: 'Use 1.2.0 instead.',
      shasum: 'def',
    },
  ],
  dist_tags: [{ tag: 'latest', version: '1.2.0' }],
}

const NPM_ADVISORIES: NpmAdvisory[] = [
  {
    id: 1001,
    url: 'https://example.com/advisory/1001',
    title: 'Prototype pollution',
    severity: 'high',
    vulnerable_versions: '<1.2.0',
    cwe: ['CWE-1321'],
    cvss_score: 7.5,
  },
]

const DEP_AUDIT: NpmDependencyAuditResult = {
  scanned_at: '2026-09-02T00:00:00Z',
  packages_scanned: 42,
  truncated: false,
  findings: [
    {
      dependency_name: 'lodash',
      dependency_version: '4.17.15',
      advisory: {
        id: 2002,
        url: 'https://example.com/advisory/2002',
        title: 'ReDoS in lodash',
        severity: 'critical',
        vulnerable_versions: '<4.17.21',
        cwe: ['CWE-1333'],
        cvss_score: 9.1,
      },
    },
  ],
}

const DOCKER_DETAILS: DockerImageDetails = {
  image_name: 'acme-api',
  tags: [
    {
      tag: 'latest',
      digest: 'sha256:abcdef0123456789',
      media_type: 'application/vnd.oci.image.manifest.v1+json',
      created_at: '2026-09-01T00:00:00Z',
    },
    {
      tag: 'v1.0.0',
      digest: 'sha256:0123456789abcdef',
      media_type: 'application/vnd.oci.image.manifest.v1+json',
      created_at: '2026-08-01T00:00:00Z',
    },
  ],
}

const IMAGE_SCAN: DockerImageScanResult = {
  scanned_at: '2026-09-02T00:00:00Z',
  vulnerabilities: [
    {
      id: 'CVE-2026-1234',
      package_name: 'openssl',
      installed_version: '3.0.1',
      fixed_version: '3.0.2',
      severity: 'CRITICAL',
      title: 'Buffer overflow',
      primary_url: 'https://example.com/cve/2026-1234',
    },
  ],
}

const REPO: RepositorySummary = {
  id: 'repo-1',
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
}

function withRoute(format: 'npm' | 'docker', name: string) {
  const params = { id: 'repo-1', format, name }
  return {
    provide: ActivatedRoute,
    useValue: {
      snapshot: { paramMap: convertToParamMap(params) },
      paramMap: of(convertToParamMap(params)),
    },
  }
}

function fakeRepositories(
  overrides: Partial<RepositoriesService> = {},
): Partial<RepositoriesService> {
  return {
    get: () => of(REPO),
    npmPackageDetails: () => of(NPM_DETAILS),
    npmPackageAudit: () => of(NPM_ADVISORIES),
    getDependencyAudit: () => of(DEP_AUDIT),
    scanDependencyTree: () => of(DEP_AUDIT),
    dockerImageDetails: () => of(DOCKER_DETAILS),
    getDockerImageScan: () => of(IMAGE_SCAN),
    scanDockerImage: () => of(IMAGE_SCAN),
    ...overrides,
  }
}

const routerStub = { provide: Router, useValue: { navigate: () => Promise.resolve(true) } }

const meta: Meta<PackageDetailPage> = {
  title: 'Repositories/PackageDetailPage',
  component: PackageDetailPage,
  decorators: [
    moduleMetadata({
      providers: [
        withRoute('npm', '@acme/ui'),
        routerStub,
        { provide: RepositoriesService, useValue: fakeRepositories() },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<PackageDetailPage>

/** An npm package with a clean security audit and no dependency findings. */
export const NpmPackage: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({
            npmPackageAudit: () => of([]),
            getDependencyAudit: () => of({ ...DEP_AUDIT, findings: [] }),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(within(canvasElement).getByText('1.2.0')).toBeInTheDocument())
  },
}

/** Known advisories on the published versions, plus a dependency-tree finding. */
export const NpmPackageWithVulnerabilities: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('Prototype pollution')).toBeInTheDocument())
    expect(canvas.getByText('lodash@4.17.15')).toBeInTheDocument()
  },
}

/** npm's advisory database being unreachable shouldn't block viewing the package itself. */
export const NpmAuditUnreachable: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({
            npmPackageAudit: () => throwError(() => new Error('down')),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() =>
      expect(
        within(canvasElement).getByText(
          "Impossible de vérifier les vulnérabilités pour le moment (base d'alertes npm injoignable).",
        ),
      ).toBeInTheDocument(),
    )
  },
}

/** Filtering dependency findings by severity narrows the list. */
export const FilteringDependencyFindingsBySeverity: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('lodash@4.17.15')).toBeInTheDocument())
    // gbt-select's trigger has no aria-label/aria-labelledby when the severity filter is
    // unlabelled — only the placeholder text as its (unnamed, per accname) content.
    await userEvent.click(canvas.getByRole('combobox'))
    await userEvent.click(await canvas.findByRole('option', { name: 'Élevée' }))
    await waitFor(() =>
      expect(
        canvas.getByText('Aucune vulnérabilité ne correspond aux criticités sélectionnées.'),
      ).toBeInTheDocument(),
    )
  },
}

/** A read-only viewer never sees delete/rescan actions. */
export const AsReadOnlyViewer: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({ get: () => of({ ...REPO, my_role: 'read' }) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('1.2.0')).toBeInTheDocument())
    expect(canvas.queryByRole('button', { name: 'Relancer le scan' })).not.toBeInTheDocument()
    expect(
      canvas.queryByRole('button', { name: 'Supprimer tout le package' }),
    ).not.toBeInTheDocument()
  },
}

/** A Docker image with a clean scan. */
export const DockerImage: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        withRoute('docker', 'acme-api'),
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({
            getDockerImageScan: () => of({ ...IMAGE_SCAN, vulnerabilities: [] }),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(within(canvasElement).getByText('latest')).toBeInTheDocument())
  },
}

/** A Docker image with a known CVE from the last scan. */
export const DockerImageWithVulnerabilities: Story = {
  decorators: [moduleMetadata({ providers: [withRoute('docker', 'acme-api')] })],
  play: async ({ canvasElement }) => {
    await waitFor(() =>
      expect(within(canvasElement).getByText('Buffer overflow')).toBeInTheDocument(),
    )
  },
}

export const DockerScanUnreachable: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        withRoute('docker', 'acme-api'),
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({
            getDockerImageScan: () => throwError(() => new Error('down')),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() =>
      expect(
        within(canvasElement).getByText(
          "Impossible d'effectuer le scan pour le moment (Trivy indisponible ou erreur d'analyse).",
        ),
      ).toBeInTheDocument(),
    )
  },
}
