import { TranslocoPipe } from '@jsverse/transloco'
import { ChangeDetectionStrategy, Component, computed, effect, inject, signal } from '@angular/core'
import { rxResource, toSignal } from '@angular/core/rxjs-interop'
import { ActivatedRoute, RouterLink } from '@angular/router'
import { LocalizedDatePipe } from '../../shared/i18n/localized-date'
import { FormsModule } from '@angular/forms'
import { HttpErrorResponse } from '@angular/common/http'
import { Button } from '@masmarino/gabarit/button'
import { Card } from '@masmarino/gabarit/card'
import { Select } from '@masmarino/gabarit/select'
import { Spinner } from '@masmarino/gabarit/spinner'
import { Tab, Tabs } from '@masmarino/gabarit/tabs'
import { RepositoriesService } from '../../repositories/application/repositories.service'
import {
  DockerImageDetails,
  NpmAdvisory,
  NpmDependencyAuditResult,
  NpmPackageDetails,
  NpmVersionDetail,
  RepositoryFormat,
} from '../../repositories/domain/repository.entity'
import { PublicLayout } from '../public-layout/public-layout'
import { PageTitleService } from '../../shell/page-title.service'
import { publicRepositoryBasePath, resolvePublicRepository } from '../public-repository-route'
import { formatSelectedCount, formatWeeklyDownloads } from '../../shared/format'
import { FormatBytesPipe } from '../../shared/format-bytes.pipe'
import { CopyableCommand } from '../../shared/copyable-command/copyable-command'
import { ReadmeView } from '../../shared/readme-view/readme-view'
import { overloadMessage } from '../../shared/api-error'
import {
  bySeverityDesc,
  buildDockerSeverityOptions,
  buildNpmSeverityOptions,
  SeverityClassPipe,
} from '../../shared/severity'
import {
  dockerPullCommand,
  npmInstallCommand,
  preferredTag,
} from '../../repositories/domain/install-commands'

const PAGE_SIZE = 20

@Component({
  selector: 'app-public-package-page',
  standalone: true,
  imports: [
    TranslocoPipe,
    Button,
    Card,
    CopyableCommand,
    LocalizedDatePipe,
    FormatBytesPipe,
    FormsModule,
    PublicLayout,
    ReadmeView,
    RouterLink,
    Select,
    SeverityClassPipe,
    Spinner,
    Tab,
    Tabs,
  ],
  templateUrl: './public-package-page.html',
  styleUrl: './public-package-page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PublicPackagePage {
  private readonly route = inject(ActivatedRoute)
  private readonly repositoriesService = inject(RepositoriesService)
  private readonly pageTitle = inject(PageTitleService)

  private readonly params = toSignal(this.route.paramMap, { requireSync: true })

  // Absolute routerLink (leading '/'), so it does not resolve relative to this page.
  readonly backLink = computed(() => publicRepositoryBasePath(this.params()))
  readonly format = computed(() => this.params().get('format') as RepositoryFormat)
  readonly packageName = computed(() => this.params().get('name')!)

  private readonly repoResource = rxResource({
    params: () => this.params(),
    stream: ({ params }) => resolvePublicRepository(this.repositoriesService, params),
  })
  private readonly repositoryId = computed(() => this.repoResource.value()?.id ?? null)

  private readonly resource = rxResource<
    NpmPackageDetails | DockerImageDetails,
    { id: string; format: RepositoryFormat; name: string } | undefined
  >({
    params: () => {
      const id = this.repositoryId()
      return id ? { id, format: this.format(), name: this.packageName() } : undefined
    },
    stream: ({ params }) =>
      params.format === 'npm'
        ? this.repositoriesService.npmPackageDetails(params.id, params.name)
        : this.repositoriesService.dockerImageDetails(params.id, params.name),
  })

  readonly loading = computed(() => this.repoResource.isLoading() || this.resource.isLoading())
  private readonly failure = computed(() => this.repoResource.error() ?? this.resource.error())
  readonly notFound = computed(() => {
    const error = this.failure()
    return !this.loading() && error instanceof HttpErrorResponse && error.status === 404
  })
  readonly loadError = computed(
    () => !this.loading() && this.failure() !== undefined && !this.notFound(),
  )
  private readonly details = computed(() =>
    !this.loading() && this.resource.hasValue() ? this.resource.value() : null,
  )
  readonly npmDetails = computed(() =>
    this.format() === 'npm' ? (this.details() as NpmPackageDetails | null) : null,
  )
  readonly dockerDetails = computed(() =>
    this.format() === 'docker' ? (this.details() as DockerImageDetails | null) : null,
  )

  readonly downloads = computed(() => {
    const count = (this.npmDetails() ?? this.dockerDetails())?.downloads_7d ?? 0
    return count > 0 ? formatWeeklyDownloads(count) : null
  })

  readonly npmCommand = computed(() => {
    const details = this.npmDetails()
    return details ? npmInstallCommand(this.packageName(), details.registry_url) : ''
  })
  readonly scannedTag = computed(() => {
    const details = this.dockerDetails()
    return details ? preferredTag(details.tags) : null
  })
  readonly dockerCommand = computed(() => {
    const details = this.dockerDetails()
    return details ? dockerPullCommand(details.image_reference, this.scannedTag()) : ''
  })

  // Same version the listing's vulnerability counts use.
  readonly latestVersion = computed(() => {
    const details = this.npmDetails()
    if (!details) {
      return null
    }
    const latestTag = details.dist_tags.find((tag) => tag.tag === 'latest')
    if (latestTag) {
      return latestTag.version
    }
    const newest = details.versions.reduce<NpmVersionDetail | null>(
      (best, candidate) =>
        best === null || Date.parse(candidate.published_at) > Date.parse(best.published_at)
          ? candidate
          : best,
      null,
    )
    return newest?.version ?? null
  })

  // Read-only: only a writer can start a scan, but anyone gets the last result of a public
  // repository.
  private readonly auditResource = rxResource({
    params: () => {
      const id = this.repositoryId()
      return id && this.format() === 'npm' ? { id, name: this.packageName() } : undefined
    },
    stream: ({ params }) => this.repositoriesService.npmPackageAudit(params.id, params.name),
  })
  readonly auditLoading = this.auditResource.isLoading
  readonly auditAdvisories = computed<NpmAdvisory[] | null>(() =>
    this.auditResource.hasValue() ? this.auditResource.value() : null,
  )
  // Anonymous visitors only read a cached advisory list (a fresh one is rate-limited to signed-in
  // accounts):
  // no cache yet is a 401 and just means nothing to show.
  readonly auditNoCache = computed(() => {
    const error = this.auditResource.error()
    return error instanceof HttpErrorResponse && error.status === 401
  })
  readonly auditFailed = computed(
    () => this.auditResource.error() !== undefined && !this.auditNoCache(),
  )
  readonly auditNotice = computed(() => overloadMessage(this.auditResource.error()))

  private readonly depAuditResource = rxResource({
    params: () => {
      const id = this.repositoryId()
      const version = this.latestVersion()
      return id && version ? { id, name: this.packageName(), version } : undefined
    },
    stream: ({ params }) =>
      this.repositoriesService.getDependencyAudit(params.id, params.name, params.version),
  })
  readonly depAuditLoading = this.depAuditResource.isLoading
  readonly depAuditResult = computed<NpmDependencyAuditResult | null>(() =>
    this.depAuditResource.hasValue() ? this.depAuditResource.value() : null,
  )
  readonly depAuditFailed = computed(() => this.depAuditResource.error() !== undefined)
  readonly depAuditNotice = computed(() => overloadMessage(this.depAuditResource.error()))
  readonly depAuditSeverityFilter = signal<string[]>([])
  readonly depAuditPage = signal(1)

  readonly sortedFindings = computed(() => {
    const result = this.depAuditResult()
    return result ? [...result.findings].sort(bySeverityDesc((f) => f.advisory.severity)) : []
  })
  readonly filteredFindings = computed(() => {
    const filter = this.depAuditSeverityFilter()
    const all = this.sortedFindings()
    return filter.length === 0 ? all : all.filter((f) => filter.includes(f.advisory.severity))
  })
  readonly depAuditTotalPages = computed(() =>
    Math.max(1, Math.ceil(this.filteredFindings().length / PAGE_SIZE)),
  )
  readonly pagedFindings = computed(() => {
    const page = Math.min(this.depAuditPage(), this.depAuditTotalPages())
    const start = (page - 1) * PAGE_SIZE
    return this.filteredFindings().slice(start, start + PAGE_SIZE)
  })

  private readonly imageScanResource = rxResource({
    params: () => {
      const id = this.repositoryId()
      const tag = this.scannedTag()
      return id && tag ? { id, name: this.packageName(), tag } : undefined
    },
    stream: ({ params }) =>
      this.repositoriesService.getDockerImageScan(params.id, params.name, params.tag),
  })
  readonly imageScanLoading = this.imageScanResource.isLoading
  readonly imageScanResult = computed(() =>
    this.imageScanResource.hasValue() ? this.imageScanResource.value() : null,
  )
  readonly imageScanFailed = computed(() => this.imageScanResource.error() !== undefined)
  readonly imageScanSeverityFilter = signal<string[]>([])
  readonly imageScanPage = signal(1)

  readonly sortedVulnerabilities = computed(() => {
    const result = this.imageScanResult()
    return result ? [...result.vulnerabilities].sort(bySeverityDesc((v) => v.severity)) : []
  })
  readonly filteredVulnerabilities = computed(() => {
    const filter = this.imageScanSeverityFilter()
    const all = this.sortedVulnerabilities()
    return filter.length === 0 ? all : all.filter((v) => filter.includes(v.severity))
  })
  readonly imageScanTotalPages = computed(() =>
    Math.max(1, Math.ceil(this.filteredVulnerabilities().length / PAGE_SIZE)),
  )
  readonly pagedVulnerabilities = computed(() => {
    const page = Math.min(this.imageScanPage(), this.imageScanTotalPages())
    const start = (page - 1) * PAGE_SIZE
    return this.filteredVulnerabilities().slice(start, start + PAGE_SIZE)
  })

  readonly selectedCountLabel = formatSelectedCount
  readonly npmSeverityOptions = buildNpmSeverityOptions()
  readonly dockerSeverityOptions = buildDockerSeverityOptions()

  constructor() {
    effect(() => this.pageTitle.title.set(this.packageName()))
    // Filter and page state do not reset on a param change.
    effect(() => {
      this.params()
      this.depAuditSeverityFilter.set([])
      this.depAuditPage.set(1)
      this.imageScanSeverityFilter.set([])
      this.imageScanPage.set(1)
    })
  }

  onDepAuditSeverityFilterChange(value: string[]): void {
    this.depAuditSeverityFilter.set(value)
    this.depAuditPage.set(1)
  }

  goToDepAuditPage(page: number): void {
    this.depAuditPage.set(page)
  }

  onImageScanSeverityFilterChange(value: string[]): void {
    this.imageScanSeverityFilter.set(value)
    this.imageScanPage.set(1)
  }

  goToImageScanPage(page: number): void {
    this.imageScanPage.set(page)
  }
}
