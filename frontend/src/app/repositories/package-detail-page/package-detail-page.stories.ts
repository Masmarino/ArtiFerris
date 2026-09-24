import { HttpErrorResponse } from '@angular/common/http'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { ActivatedRoute, Router, convertToParamMap } from '@angular/router'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { NEVER, of, throwError } from 'rxjs'
import { PackageDetailPage } from './package-detail-page'
import { RepositoriesService } from '../application/repositories.service'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'
import { RICH_README_HTML } from '../../shared/readme-view/readme.fixtures'
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
  truncated: false,
  dist_tags: [{ tag: 'latest', version: '1.2.0' }],
  readme_html: RICH_README_HTML,
  registry_url: 'http://localhost:4200/npm/u/alice/acme-npm/',
  downloads_7d: 1234,
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
  image_reference: 'localhost:4200/o/acme/acme-docker/acme-api',
  downloads_7d: 56,
  truncated: false,
  tags: [
    {
      tag: 'latest',
      digest: 'sha256:abcdef0123456789',
      media_type: 'application/vnd.oci.image.manifest.v1+json',
      created_at: '2026-09-01T00:00:00Z',
      size_bytes: 48_234_496,
    },
    {
      tag: 'v1.0.0',
      digest: 'sha256:0123456789abcdef',
      media_type: 'application/vnd.oci.image.manifest.v1+json',
      created_at: '2026-08-01T00:00:00Z',
      size_bytes: null,
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
  is_public: false,
  my_role: 'admin',
  organization_id: 'org-acme',
  owner_name: 'Acme Corp',
  owner_is_personal: false,
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
    expect(
      within(canvasElement).getByText(/^1\s234 téléchargements cette semaine$/),
    ).toBeInTheDocument()
  },
}

/** No download this week: the line is not drawn. */
export const NpmPackageWithoutDownloads: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({
            npmPackageDetails: () => of({ ...NPM_DETAILS, downloads_7d: 0 }),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('1.2.0')).toBeInTheDocument())
    expect(canvas.queryByText(/téléchargement/)).toBeNull()
  },
}

/** The install command comes from the owner's registry URL; the README sits between the versions and the security card. */
/** The backend only sent the newest 200 versions. */
export const NpmPackageWithTruncatedVersions: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({
            npmPackageDetails: () => of({ ...NPM_DETAILS, truncated: true }),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    expect(
      await within(canvasElement).findByText(
        'Seules les 200 versions les plus récentes sont affichées.',
      ),
    ).toBeInTheDocument()
  },
}

/** The backend only sent the newest 100 tags. */
export const DockerImageWithTruncatedTags: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        withRoute('docker', 'acme-api'),
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({
            dockerImageDetails: () => of({ ...DOCKER_DETAILS, truncated: true }),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    expect(
      await within(canvasElement).findByText('Seuls les 100 tags les plus récents sont affichés.'),
    ).toBeInTheDocument()
  },
}

export const NpmPackageWithReadme: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByRole('heading', { name: 'README' })).toBeInTheDocument())
    expect(canvasElement.querySelector('app-copyable-command pre')).toHaveTextContent(
      'npm install @acme/ui --registry http://localhost:4200/npm/u/alice/acme-npm/',
    )
    expect(canvas.getByRole('heading', { name: 'Usage' })).toBeInTheDocument()
  },
}

export const NpmPackageWithoutReadme: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({
            npmPackageDetails: () => of({ ...NPM_DETAILS, readme_html: null }),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(within(canvasElement).getByText('Aucun README')).toBeInTheDocument())
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

/** A 429 from the audit route is the server limiting requests, not npm being down. */
export const NpmAuditRateLimited: Story = {
  decorators: [
    withRepositories({
      npmPackageAudit: () => throwError(() => new HttpErrorResponse({ status: 429 })),
    }),
  ],
  play: async ({ canvasElement }) => {
    expect(
      await within(canvasElement).findByText('Trop de demandes, réessayez dans un instant'),
    ).toBeInTheDocument()
  },
}

export const NpmAuditBusy: Story = {
  decorators: [
    withRepositories({
      npmPackageAudit: () => throwError(() => new HttpErrorResponse({ status: 503 })),
    }),
  ],
  play: async ({ canvasElement }) => {
    expect(
      await within(canvasElement).findByText('Service momentanément occupé'),
    ).toBeInTheDocument()
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
    expect(within(canvasElement).getByText('56 téléchargements cette semaine')).toBeInTheDocument()
  },
}

export const DockerImageWithoutDownloads: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        withRoute('docker', 'acme-api'),
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({
            dockerImageDetails: () => of({ ...DOCKER_DETAILS, downloads_7d: 0 }),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('latest')).toBeInTheDocument())
    expect(canvas.queryByText(/téléchargement/)).toBeNull()
  },
}

/** The pull command targets the latest tag; each tag shows its size ("—" when unknown). */
export const DockerImageWithPullCommandAndSizes: Story = {
  decorators: [moduleMetadata({ providers: [withRoute('docker', 'acme-api')] })],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByRole('cell', { name: '46.0 Mo' })).toBeInTheDocument())
    expect(canvasElement.querySelector('app-copyable-command pre')).toHaveTextContent(
      'docker pull localhost:4200/o/acme/acme-docker/acme-api:latest',
    )
    expect(canvas.getByRole('columnheader', { name: 'Taille' })).toBeInTheDocument()
    expect(canvas.getByRole('cell', { name: '—' })).toBeInTheDocument()
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

function withRouterSpy() {
  const navigate = fn(() => Promise.resolve(true))
  return {
    navigate,
    decorator: moduleMetadata({ providers: [{ provide: Router, useValue: { navigate } }] }),
  }
}
function withRepositories(overrides: Partial<RepositoriesService>) {
  return moduleMetadata({
    providers: [{ provide: RepositoriesService, useValue: fakeRepositories(overrides) }],
  })
}
function withToastSpy() {
  const toast = { success: fn(), error: fn() }
  return {
    toast,
    decorator: moduleMetadata({ providers: [{ provide: ToastService, useValue: toast }] }),
  }
}
async function deleteButtonOf(canvas: ReturnType<typeof within>, version: string) {
  return within(await canvas.findByTestId(`delete-version-${version}`)).getByRole('button')
}
function withConfirm(answer: boolean) {
  const ask = fn(() => Promise.resolve(answer))
  return {
    ask,
    decorator: moduleMetadata({ providers: [{ provide: ConfirmService, useValue: { ask } }] }),
  }
}

export const Loading: Story = {
  decorators: [withRepositories({ npmPackageDetails: () => NEVER })],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('status')).toHaveTextContent('Chargement…')
    expect(canvas.queryByText('1.2.0')).not.toBeInTheDocument()
  },
}

export const AuditLoading: Story = {
  decorators: [withRepositories({ npmPackageAudit: () => NEVER })],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('status')).toHaveTextContent(
      'Vérification des vulnérabilités connues…',
    )
  },
}

export const DependencyAuditLoading: Story = {
  decorators: [withRepositories({ getDependencyAudit: () => NEVER })],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('status')).toHaveTextContent('Chargement du dernier résultat…')
    expect(canvas.queryByText('Aucune analyse')).not.toBeInTheDocument()
  },
}

export const DockerScanLoading: Story = {
  decorators: [
    moduleMetadata({ providers: [withRoute('docker', 'acme-api')] }),
    withRepositories({ getDockerImageScan: () => NEVER }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('status')).toHaveTextContent('Chargement du dernier résultat…')
    expect(canvas.queryByText('Aucun scan')).not.toBeInTheDocument()
  },
}

const npmLoadFails = withRouterSpy()
/** A package that can't be loaded sends the user back to its repository. */
export const NpmLoadFailed: Story = {
  decorators: [
    npmLoadFails.decorator,
    withRepositories({ npmPackageDetails: () => throwError(() => new Error('down')) }),
  ],
  play: async () => {
    await waitFor(() =>
      expect(npmLoadFails.navigate).toHaveBeenCalledWith(['/repositories', 'repo-1']),
    )
  },
}

const dockerLoadFails = withRouterSpy()
export const DockerLoadFailed: Story = {
  decorators: [
    moduleMetadata({ providers: [withRoute('docker', 'acme-api')] }),
    dockerLoadFails.decorator,
    withRepositories({ dockerImageDetails: () => throwError(() => new Error('down')) }),
  ],
  play: async () => {
    await waitFor(() =>
      expect(dockerLoadFails.navigate).toHaveBeenCalledWith(['/repositories', 'repo-1']),
    )
  },
}

const dockerEmpty = withRouterSpy()
/** An image whose last tag was deleted has nothing left to manage, so the page leaves. */
export const DockerImageWithoutTags: Story = {
  decorators: [
    moduleMetadata({ providers: [withRoute('docker', 'acme-api')] }),
    dockerEmpty.decorator,
    withRepositories({ dockerImageDetails: () => of({ ...DOCKER_DETAILS, tags: [] }) }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() =>
      expect(dockerEmpty.navigate).toHaveBeenCalledWith(['/repositories', 'repo-1']),
    )
    expect(within(canvasElement).queryByRole('table')).not.toBeInTheDocument()
  },
}

const depScan = withRepositories({
  getDependencyAudit: fn(() => of(null)),
  scanDependencyTree: fn(() => of(DEP_AUDIT)),
})
/** No scan has ever run on the latest version; a writer can start one. */
export const DependencyScanNeverRun: Story = {
  decorators: [depScan],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Aucune analyse')).toBeInTheDocument()
    const buttons = canvas.getAllByRole('button', { name: 'Lancer un scan approfondi' })
    await userEvent.click(buttons[buttons.length - 1])
    expect(await canvas.findByText('lodash@4.17.15')).toBeInTheDocument()
    expect(canvas.queryByText('Aucune analyse')).not.toBeInTheDocument()
  },
}

export const DependencyAuditFailed: Story = {
  decorators: [withRepositories({ getDependencyAudit: () => throwError(() => new Error('down')) })],
  play: async ({ canvasElement }) => {
    expect(
      await within(canvasElement).findByText(
        "Impossible d'effectuer l'analyse pour le moment (base d'alertes npm injoignable).",
      ),
    ).toBeInTheDocument()
  },
}

/** Dependency trees too big to cover entirely are flagged as partially scanned. */
export const DependencyAuditTruncated: Story = {
  decorators: [
    withRepositories({ getDependencyAudit: () => of({ ...DEP_AUDIT, truncated: true }) }),
  ],
  play: async ({ canvasElement }) => {
    expect(await within(canvasElement).findByText(/analyse partielle/)).toBeInTheDocument()
  },
}

const imageScan = withRepositories({
  getDockerImageScan: fn(() => of(null)),
  scanDockerImage: fn(() => of(IMAGE_SCAN)),
})
export const DockerScanNeverRun: Story = {
  decorators: [moduleMetadata({ providers: [withRoute('docker', 'acme-api')] }), imageScan],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Aucun scan')).toBeInTheDocument()
    const buttons = canvas.getAllByRole('button', { name: 'Lancer un scan' })
    await userEvent.click(buttons[buttons.length - 1])
    expect(await canvas.findByText('Buffer overflow')).toBeInTheDocument()
    expect(canvas.queryByText('Aucun scan')).not.toBeInTheDocument()
  },
}

export const DockerImageAsReadOnlyViewer: Story = {
  decorators: [
    moduleMetadata({ providers: [withRoute('docker', 'acme-api')] }),
    withRepositories({ get: () => of({ ...REPO, my_role: 'read' }) }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('latest')).toBeInTheDocument())
    expect(canvas.queryByRole('button', { name: 'Supprimer' })).not.toBeInTheDocument()
    expect(canvas.queryByRole('button', { name: 'Lancer un scan' })).not.toBeInTheDocument()
    expect(
      canvas.queryByRole('button', { name: "Supprimer toute l'image" }),
    ).not.toBeInTheDocument()
  },
}

const manyVersions = Array.from({ length: 25 }, (_, i) => ({
  version: `1.0.${24 - i}`,
  published_at: '2026-09-01T00:00:00Z',
  size_bytes: 1_000,
  deprecated: false,
  deprecated_message: null,
  shasum: `sha-${i}`,
}))
/** Version history is paged 20 at a time. */
export const PaginatedVersionHistory: Story = {
  decorators: [
    withRepositories({
      npmPackageDetails: () => of({ ...NPM_DETAILS, versions: manyVersions }),
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('25 versions')).toBeInTheDocument()
    expect(canvas.getByText('Page 1 sur 2')).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Précédent' })).toBeDisabled()
    expect(canvas.getByText('1.0.24')).toBeInTheDocument()
    expect(canvas.queryByText('1.0.4')).not.toBeInTheDocument()
    await userEvent.click(canvas.getByRole('button', { name: 'Suivant' }))
    expect(await canvas.findByText('Page 2 sur 2')).toBeInTheDocument()
    expect(canvas.getByText('1.0.4')).toBeInTheDocument()
    expect(canvas.queryByText('1.0.24')).not.toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Suivant' })).toBeDisabled()
  },
}

const deleteOk = withToastSpy()
const deleteOkConfirm = withConfirm(true)
const deleteOkRepositories = { deleteNpmPackageVersion: fn(() => of(undefined)) }
export const DeletingAVersion: Story = {
  decorators: [
    deleteOk.decorator,
    deleteOkConfirm.decorator,
    withRepositories(deleteOkRepositories),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const button = await deleteButtonOf(canvas, '1.1.0')
    await userEvent.click(button)
    expect(deleteOkConfirm.ask).toHaveBeenCalledWith(
      expect.objectContaining({
        heading: 'Supprimer la version',
        message: 'Supprimer la version 1.1.0 de @acme/ui ?',
        danger: true,
      }),
    )
    await waitFor(() =>
      expect(deleteOkRepositories.deleteNpmPackageVersion).toHaveBeenCalledWith(
        'repo-1',
        '@acme/ui',
        '1.1.0',
      ),
    )
    expect(deleteOk.toast.success).toHaveBeenCalledWith('Version 1.1.0 supprimée.')
  },
}

const deleteDeclined = withToastSpy()
const deleteDeclinedConfirm = withConfirm(false)
const deleteDeclinedRepositories = { deleteNpmPackageVersion: fn(() => of(undefined)) }
export const DeletingAVersionDeclined: Story = {
  decorators: [
    deleteDeclined.decorator,
    deleteDeclinedConfirm.decorator,
    withRepositories(deleteDeclinedRepositories),
  ],
  play: async ({ canvasElement }) => {
    const button = await deleteButtonOf(within(canvasElement), '1.1.0')
    await userEvent.click(button)
    await waitFor(() => expect(deleteDeclinedConfirm.ask).toHaveBeenCalledOnce())
    expect(deleteDeclinedRepositories.deleteNpmPackageVersion).not.toHaveBeenCalled()
    expect(deleteDeclined.toast.success).not.toHaveBeenCalled()
  },
}

const deleteFails = withToastSpy()
const deleteFailsConfirm = withConfirm(true)
export const DeletingAVersionFailed: Story = {
  decorators: [
    deleteFails.decorator,
    deleteFailsConfirm.decorator,
    withRepositories({ deleteNpmPackageVersion: () => throwError(() => new Error('down')) }),
  ],
  play: async ({ canvasElement }) => {
    const button = await deleteButtonOf(within(canvasElement), '1.1.0')
    await userEvent.click(button)
    await waitFor(() =>
      expect(deleteFails.toast.error).toHaveBeenCalledWith(
        'Échec de la suppression de la version 1.1.0.',
      ),
    )
  },
}
