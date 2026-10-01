import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  Pipe,
  PipeTransform,
  computed,
  effect,
  inject,
  signal,
} from '@angular/core'
import { toSignal } from '@angular/core/rxjs-interop'
import { ActivatedRoute, Router, RouterLink } from '@angular/router'
import { LocalizedDatePipe } from '../../shared/i18n/localized-date'
import { FormsModule } from '@angular/forms'
import { map } from 'rxjs'
import { Button, Card, EmptyState, Select, Spinner } from '@masmarino/gabarit'
import { RepositoriesService } from '../application/repositories.service'
import {
  DockerImageDetails,
  DockerImageScanResult,
  NpmAdvisory,
  NpmDependencyAuditResult,
  NpmPackageDetails,
  NpmVersionDetail,
  RepositoryFormat,
} from '../domain/repository.entity'
import { PageTitleService } from '../../shell/page-title.service'
import { FormatBytesPipe } from '../../shared/format-bytes.pipe'
import {
  bySeverityDesc,
  buildDockerSeverityOptions,
  buildNpmSeverityOptions,
  SeverityClassPipe,
} from '../../shared/severity'
import { formatSelectedCount, formatWeeklyDownloads } from '../../shared/format'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'
import { overloadMessage } from '../../shared/api-error'
import { CopyableCommand } from '../../shared/copyable-command/copyable-command'
import { ReadmeView } from '../../shared/readme-view/readme-view'
import { dockerPullCommand, npmInstallCommand, preferredTag } from '../domain/install-commands'

const PAGE_SIZE = 20

@Pipe({ name: 'shortDigest' })
class ShortDigestPipe implements PipeTransform {
  transform(digest: string): string {
    const [algorithm, hex] = digest.split(':')
    return hex ? `${algorithm}:${hex.slice(0, 12)}…` : digest
  }
}

@Component({
  selector: 'app-package-detail-page',
  standalone: true,
  imports: [
    TranslocoPipe,
    Button,
    LocalizedDatePipe,
    RouterLink,
    Card,
    CopyableCommand,
    EmptyState,
    ReadmeView,
    Select,
    Spinner,
    FormsModule,
    FormatBytesPipe,
    ShortDigestPipe,
    SeverityClassPipe,
  ],
  templateUrl: './package-detail-page.html',
  styleUrl: './package-detail-page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PackageDetailPage {
  private readonly route = inject(ActivatedRoute)
  private readonly router = inject(Router)
  private readonly repositoriesService = inject(RepositoriesService)
  private readonly pageTitle = inject(PageTitleService)
  private readonly toastService = inject(ToastService)
  private readonly confirmService = inject(ConfirmService)

  // Reactive, not route.snapshot: Angular reuses this component across navigations.
  private readonly routeParams = toSignal(
    this.route.paramMap.pipe(
      map((params) => ({
        repositoryId: params.get('id')!,
        format: params.get('format') as RepositoryFormat,
        name: params.get('name')!,
      })),
    ),
    { requireSync: true },
  )

  repositoryId!: string
  format!: RepositoryFormat
  name!: string

  readonly selectedCountLabel = formatSelectedCount

  readonly loading = signal(true)
  readonly npmDetails = signal<NpmPackageDetails | null>(null)
  readonly dockerDetails = signal<DockerImageDetails | null>(null)

  readonly versionsPage = signal(1)
  readonly versionsTotalPages = computed(() =>
    Math.max(1, Math.ceil((this.npmDetails()?.versions.length ?? 0) / PAGE_SIZE)),
  )
  readonly pagedVersions = computed(() => {
    const details = this.npmDetails()
    if (!details) {
      return []
    }
    const page = Math.min(this.versionsPage(), this.versionsTotalPages())
    const start = (page - 1) * PAGE_SIZE
    return details.versions.slice(start, start + PAGE_SIZE)
  })

  readonly tagsPage = signal(1)
  readonly tagsTotalPages = computed(() =>
    Math.max(1, Math.ceil((this.dockerDetails()?.tags.length ?? 0) / PAGE_SIZE)),
  )
  readonly pagedTags = computed(() => {
    const details = this.dockerDetails()
    if (!details) {
      return []
    }
    const page = Math.min(this.tagsPage(), this.tagsTotalPages())
    const start = (page - 1) * PAGE_SIZE
    return details.tags.slice(start, start + PAGE_SIZE)
  })

  // Fetched once so write buttons can be hidden for a read-only viewer.
  readonly canWrite = signal(false)

  // An npm advisory outage must not block viewing the package.
  readonly auditLoading = signal(true)
  readonly auditAdvisories = signal<NpmAdvisory[] | null>(null)
  readonly auditFailed = signal(false)
  readonly auditNotice = signal<string | null>(null)

  // Reads the last saved result for `latest`; a fresh scan needs a click.
  readonly depAuditLoading = signal(true)
  readonly depAuditResult = signal<NpmDependencyAuditResult | null>(null)
  readonly depAuditFailed = signal(false)
  readonly depAuditNotice = signal<string | null>(null)
  readonly depAuditScanning = signal(false)
  readonly depAuditSeverityFilter = signal<string[]>([])
  readonly depAuditPage = signal(1)
  readonly npmSeverityOptions = buildNpmSeverityOptions()

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

  // Reads the last completed Trivy scan, not a live one.
  readonly imageScanLoading = signal(true)
  readonly imageScanResult = signal<DockerImageScanResult | null>(null)
  readonly imageScanFailed = signal(false)
  readonly imageScanScanning = signal(false)
  readonly imageScanSeverityFilter = signal<string[]>([])
  readonly imageScanPage = signal(1)
  readonly dockerSeverityOptions = buildDockerSeverityOptions()

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

  constructor() {
    effect(() => this.pageTitle.title.set(this.routeParams().name))
    effect(() => {
      const params = this.routeParams()
      this.repositoryId = params.repositoryId
      this.format = params.format
      this.name = params.name
      this.resetPackageState()
      this.reload()
      const stillCurrent = this.stillCurrentGuard()
      this.repositoriesService.get(this.repositoryId).subscribe({
        next: (repo) => {
          if (!stillCurrent()) {
            return
          }
          this.canWrite.set(repo.my_role === 'write' || repo.my_role === 'admin')
        },
        error: () => {
          // canWrite stays false: the buttons are a convenience, the backend re-checks the role.
        },
      })
    })
  }

  // Reused across navigations: drop the previous data.
  private resetPackageState(): void {
    this.npmDetails.set(null)
    this.dockerDetails.set(null)
    this.canWrite.set(false)
    this.versionsPage.set(1)
    this.tagsPage.set(1)
    this.auditLoading.set(true)
    this.auditAdvisories.set(null)
    this.auditFailed.set(false)
    this.auditNotice.set(null)
    this.depAuditLoading.set(true)
    this.depAuditResult.set(null)
    this.depAuditFailed.set(false)
    this.depAuditNotice.set(null)
    this.depAuditScanning.set(false)
    this.depAuditSeverityFilter.set([])
    this.depAuditPage.set(1)
    this.imageScanLoading.set(true)
    this.imageScanResult.set(null)
    this.imageScanFailed.set(false)
    this.imageScanScanning.set(false)
    this.imageScanSeverityFilter.set([])
    this.imageScanPage.set(1)
  }

  private stillCurrentGuard(): () => boolean {
    const requested = { repositoryId: this.repositoryId, format: this.format, name: this.name }
    return () => {
      const current = this.routeParams()
      return (
        current.repositoryId === requested.repositoryId &&
        current.format === requested.format &&
        current.name === requested.name
      )
    }
  }

  private reload(): void {
    this.loading.set(true)
    const stillCurrent = this.stillCurrentGuard()
    if (this.format === 'npm') {
      this.repositoriesService.npmPackageDetails(this.repositoryId, this.name).subscribe({
        next: (details) => {
          if (!stillCurrent()) {
            return
          }
          this.npmDetails.set(details)
          this.versionsPage.set(1)
          this.loading.set(false)
          this.loadAudit()
          this.loadDependencyAudit()
        },
        error: () => {
          if (stillCurrent()) {
            this.backToRepository()
          }
        },
      })
    } else {
      this.repositoriesService.dockerImageDetails(this.repositoryId, this.name).subscribe({
        next: (details) => {
          if (!stillCurrent()) {
            return
          }
          // Deleting the last tag leaves nothing to manage.
          if (details.tags.length === 0) {
            this.backToRepository()
            return
          }
          this.dockerDetails.set(details)
          this.tagsPage.set(1)
          this.loading.set(false)
          this.loadImageScan()
        },
        error: () => {
          if (stillCurrent()) {
            this.backToRepository()
          }
        },
      })
    }
  }

  rescan(): void {
    this.loadAudit()
  }

  private loadAudit(): void {
    this.auditLoading.set(true)
    this.auditFailed.set(false)
    this.auditNotice.set(null)
    const stillCurrent = this.stillCurrentGuard()
    this.repositoriesService.npmPackageAudit(this.repositoryId, this.name).subscribe({
      next: (advisories) => {
        if (!stillCurrent()) {
          return
        }
        this.auditAdvisories.set(advisories)
        this.auditLoading.set(false)
      },
      error: (error: unknown) => {
        if (!stillCurrent()) {
          return
        }
        this.auditNotice.set(overloadMessage(error))
        this.auditFailed.set(true)
        this.auditLoading.set(false)
      },
    })
  }

  readonly latestVersion = computed(() => {
    const details = this.npmDetails()
    if (!details) {
      return null
    }
    const latestTag = details.dist_tags.find((tag) => tag.tag === 'latest')
    if (latestTag) {
      return latestTag.version
    }
    // No `latest` tag: the newest version, whatever the list order.
    const newest = details.versions.reduce<NpmVersionDetail | null>(
      (best, candidate) =>
        best === null || Date.parse(candidate.published_at) > Date.parse(best.published_at)
          ? candidate
          : best,
      null,
    )
    return newest?.version ?? null
  })

  private loadDependencyAudit(): void {
    const version = this.latestVersion()
    if (!version) {
      this.depAuditLoading.set(false)
      return
    }
    this.depAuditLoading.set(true)
    this.depAuditFailed.set(false)
    this.depAuditNotice.set(null)
    const stillCurrent = this.stillCurrentGuard()
    this.repositoriesService.getDependencyAudit(this.repositoryId, this.name, version).subscribe({
      next: (result) => {
        if (!stillCurrent()) {
          return
        }
        this.depAuditResult.set(result)
        this.depAuditPage.set(1)
        this.depAuditLoading.set(false)
      },
      error: (error: unknown) => {
        if (!stillCurrent()) {
          return
        }
        this.depAuditNotice.set(overloadMessage(error))
        this.depAuditFailed.set(true)
        this.depAuditLoading.set(false)
      },
    })
  }

  runDependencyScan(): void {
    const version = this.latestVersion()
    if (!version) {
      return
    }
    this.depAuditScanning.set(true)
    this.depAuditFailed.set(false)
    this.depAuditNotice.set(null)
    const stillCurrent = this.stillCurrentGuard()
    this.repositoriesService.scanDependencyTree(this.repositoryId, this.name, version).subscribe({
      next: (result) => {
        if (!stillCurrent()) {
          return
        }
        this.depAuditResult.set(result)
        this.depAuditPage.set(1)
        this.depAuditScanning.set(false)
      },
      error: (error: unknown) => {
        if (!stillCurrent()) {
          return
        }
        this.depAuditNotice.set(overloadMessage(error))
        this.depAuditFailed.set(true)
        this.depAuditScanning.set(false)
      },
    })
  }

  onDepAuditSeverityFilterChange(value: string[]): void {
    this.depAuditSeverityFilter.set(value)
    this.depAuditPage.set(1)
  }

  goToDepAuditPage(page: number): void {
    this.depAuditPage.set(page)
  }

  readonly scannedTag = computed(() => {
    const details = this.dockerDetails()
    return details ? preferredTag(details.tags) : null
  })

  readonly downloads = computed(() => {
    const count = (this.npmDetails() ?? this.dockerDetails())?.downloads_7d ?? 0
    return count > 0 ? formatWeeklyDownloads(count) : null
  })

  readonly npmCommand = computed(() => {
    const details = this.npmDetails()
    return details ? npmInstallCommand(details.name, details.registry_url) : ''
  })

  readonly dockerCommand = computed(() => {
    const details = this.dockerDetails()
    return details ? dockerPullCommand(details.image_reference, this.scannedTag()) : ''
  })

  private loadImageScan(): void {
    const tag = this.scannedTag()
    if (!tag) {
      this.imageScanLoading.set(false)
      return
    }
    this.imageScanLoading.set(true)
    this.imageScanFailed.set(false)
    const stillCurrent = this.stillCurrentGuard()
    this.repositoriesService.getDockerImageScan(this.repositoryId, this.name, tag).subscribe({
      next: (result) => {
        if (!stillCurrent()) {
          return
        }
        this.imageScanResult.set(result)
        this.imageScanPage.set(1)
        this.imageScanLoading.set(false)
      },
      error: () => {
        if (!stillCurrent()) {
          return
        }
        this.imageScanFailed.set(true)
        this.imageScanLoading.set(false)
      },
    })
  }

  runImageScan(): void {
    const tag = this.scannedTag()
    if (!tag) {
      return
    }
    this.imageScanScanning.set(true)
    this.imageScanFailed.set(false)
    const stillCurrent = this.stillCurrentGuard()
    this.repositoriesService.scanDockerImage(this.repositoryId, this.name, tag).subscribe({
      next: (result) => {
        if (!stillCurrent()) {
          return
        }
        this.imageScanResult.set(result)
        this.imageScanPage.set(1)
        this.imageScanScanning.set(false)
      },
      error: () => {
        if (!stillCurrent()) {
          return
        }
        this.imageScanFailed.set(true)
        this.imageScanScanning.set(false)
      },
    })
  }

  onImageScanSeverityFilterChange(value: string[]): void {
    this.imageScanSeverityFilter.set(value)
    this.imageScanPage.set(1)
  }

  goToImageScanPage(page: number): void {
    this.imageScanPage.set(page)
  }

  goToVersionsPage(page: number): void {
    this.versionsPage.set(page)
  }

  goToTagsPage(page: number): void {
    this.tagsPage.set(page)
  }

  backToRepository(): void {
    this.router.navigate(['/repositories', this.repositoryId])
  }

  async deleteVersion(version: string): Promise<void> {
    const confirmed = await this.confirmService.ask({
      heading: t('package.delete.versionHeading'),
      message: t('package.delete.versionMessage', { version, name: this.name }),
      confirmLabel: t('common.delete'),
      danger: true,
    })
    if (!confirmed) {
      return
    }
    this.repositoriesService
      .deleteNpmPackageVersion(this.repositoryId, this.name, version)
      .subscribe({
        next: () => {
          this.reload()
          this.toastService.success(t('package.delete.versionDeleted', { version }))
        },
        error: () => this.toastService.error(t('package.delete.versionFailed', { version })),
      })
  }

  async deleteWholePackage(): Promise<void> {
    const confirmed = await this.confirmService.ask({
      heading: t('package.delete.packageHeading'),
      message: t('package.delete.packageMessage', { name: this.name }),
      confirmLabel: t('common.delete'),
      danger: true,
      typeToConfirm: this.name,
    })
    if (!confirmed) {
      return
    }
    this.repositoriesService.deleteNpmPackage(this.repositoryId, this.name).subscribe({
      next: () => {
        this.backToRepository()
        this.toastService.success(t('package.delete.packageDeleted', { name: this.name }))
      },
      error: () => this.toastService.error(t('package.delete.packageFailed')),
    })
  }

  async deleteTag(tag: string): Promise<void> {
    const confirmed = await this.confirmService.ask({
      heading: t('package.delete.tagHeading'),
      message: t('package.delete.tagMessage', { tag, name: this.name }),
      confirmLabel: t('common.delete'),
      danger: true,
    })
    if (!confirmed) {
      return
    }
    this.repositoriesService.deleteDockerTag(this.repositoryId, this.name, tag).subscribe({
      next: () => {
        this.reload()
        this.toastService.success(t('package.delete.tagDeleted', { tag }))
      },
      error: () => this.toastService.error(t('package.delete.tagFailed', { tag })),
    })
  }

  async deleteWholeImage(): Promise<void> {
    const confirmed = await this.confirmService.ask({
      heading: t('package.delete.imageHeading'),
      message: t('package.delete.imageMessage', { name: this.name }),
      confirmLabel: t('common.delete'),
      danger: true,
      typeToConfirm: this.name,
    })
    if (!confirmed) {
      return
    }
    this.repositoriesService.deleteDockerImage(this.repositoryId, this.name).subscribe({
      next: () => {
        this.backToRepository()
        this.toastService.success(t('package.delete.imageDeleted', { name: this.name }))
      },
      error: () => this.toastService.error(t('package.delete.imageFailed')),
    })
  }
}
