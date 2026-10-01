import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  OnInit,
  computed,
  effect,
  inject,
  output,
  signal,
} from '@angular/core'
import { toSignal } from '@angular/core/rxjs-interop'
import {
  FormControl,
  FormGroup,
  FormsModule,
  ReactiveFormsModule,
  Validators,
} from '@angular/forms'
import { Button, Checkbox, GbtInput, Modal, Select, type SelectOption } from '@masmarino/gabarit'
import { RepositoriesService } from '../application/repositories.service'
import { RepositoryFormat, RepositorySummary, RepositoryType } from '../domain/repository.entity'
import { ToastService } from '../../shared/toast.service'
import { MeService } from '../../shell/application/me.service'
import { rejectionMessage } from '../../shared/api-error'

const FORMAT_OPTIONS: SelectOption<RepositoryFormat>[] = [
  { value: 'npm', label: 'npm' },
  { value: 'docker', label: 'docker' },
]

const REPO_TYPE_OPTIONS: SelectOption<RepositoryType>[] = [
  { value: 'hosted', label: 'hosted' },
  { value: 'proxy', label: 'proxy' },
  { value: 'group', label: 'group' },
]

const BYTES_PER_MB = 1024 * 1024

@Component({
  selector: 'app-create-repository-modal',
  standalone: true,
  imports: [
    TranslocoPipe,
    ReactiveFormsModule,
    FormsModule,
    Modal,
    GbtInput,
    Select,
    Checkbox,
    Button,
  ],
  templateUrl: './create-repository-modal.html',
  styleUrl: './create-repository-modal.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class CreateRepositoryModal implements OnInit {
  private readonly repositoriesService = inject(RepositoriesService)
  private readonly toastService = inject(ToastService)
  private readonly me = inject(MeService)

  readonly created = output<void>()
  readonly cancelled = output<void>()

  readonly formatOptions = FORMAT_OPTIONS
  readonly repoTypeOptions = REPO_TYPE_OPTIONS

  // A super-admin using this UI always creates in the public organization (requests resolve to it:
  // no subdomain awareness),
  // so the super-admin check is enough.
  readonly isSuperAdmin = computed(() => this.me.isSuperAdmin())

  readonly form = new FormGroup({
    name: new FormControl('', { nonNullable: true, validators: [Validators.required] }),
    format: new FormControl<RepositoryFormat>('npm', { nonNullable: true }),
    repoType: new FormControl<RepositoryType>('hosted', { nonNullable: true }),
    remoteUrl: new FormControl('', { nonNullable: true }),
    remoteUsername: new FormControl('', { nonNullable: true }),
    remotePassword: new FormControl('', { nonNullable: true }),
    quotaMb: new FormControl('', { nonNullable: true }),
    retentionKeepLastN: new FormControl('', { nonNullable: true }),
    isPublic: new FormControl(false, { nonNullable: true }),
  })

  // Zoneless: only signals trigger a re-render, so the FormControl can't be read directly.
  private readonly format = toSignal(this.form.controls.format.valueChanges, {
    initialValue: this.form.controls.format.value,
  })
  private readonly repoType = toSignal(this.form.controls.repoType.valueChanges, {
    initialValue: this.form.controls.repoType.value,
  })
  readonly isProxy = computed(() => this.repoType() === 'proxy')
  readonly isGroup = computed(() => this.repoType() === 'group')
  // A public proxy would relay anonymously with stored credentials and a public group would expose
  // every member's visibility:
  // the backend rejects both.
  readonly canBePublic = computed(() => !this.isProxy() && !this.isGroup())

  readonly allRepositories = signal<RepositorySummary[]>([])
  readonly selectedMemberId = signal('')
  readonly groupMembers = signal<RepositorySummary[]>([])

  readonly availableMemberOptions = computed<SelectOption<string>[]>(() => {
    const chosen = new Set(this.groupMembers().map((r) => r.id))
    return this.allRepositories()
      .filter((r) => r.format === this.format() && !chosen.has(r.id))
      .map((r) => ({ value: r.id, label: r.name }))
  })

  constructor() {
    // A group holds one format, so drop stale picks when it changes.
    effect(() => {
      this.format()
      this.groupMembers.set([])
    })
    // Drop a stale checked state so it is not submitted while the checkbox is hidden.
    effect(() => {
      if (!this.canBePublic()) {
        this.form.controls.isPublic.setValue(false)
      }
    })
  }

  ngOnInit(): void {
    this.repositoriesService.list().subscribe((repos) => this.allRepositories.set(repos))
  }

  addMember(): void {
    const id = this.selectedMemberId()
    const repo = this.allRepositories().find((r) => r.id === id)
    if (!repo) {
      return
    }
    this.groupMembers.update((members) => [...members, repo])
    this.selectedMemberId.set('')
  }

  removeMember(id: string): void {
    this.groupMembers.update((members) => members.filter((m) => m.id !== id))
  }

  moveMemberUp(index: number): void {
    if (index <= 0) {
      return
    }
    this.groupMembers.update((members) => {
      const reordered = [...members]
      ;[reordered[index - 1], reordered[index]] = [reordered[index], reordered[index - 1]]
      return reordered
    })
  }

  moveMemberDown(index: number): void {
    this.groupMembers.update((members) => {
      if (index >= members.length - 1) {
        return members
      }
      const reordered = [...members]
      ;[reordered[index + 1], reordered[index]] = [reordered[index], reordered[index + 1]]
      return reordered
    })
  }

  get quotaError(): string | null {
    const raw = this.form.controls.quotaMb.value.trim()
    if (raw === '') {
      return null
    }
    const mb = Number(raw)
    return Number.isFinite(mb) && mb >= 0 ? null : t('repositories.create.errors.quota')
  }

  get retentionError(): string | null {
    const raw = this.form.controls.retentionKeepLastN.value.trim()
    if (raw === '') {
      return null
    }
    const n = Number(raw)
    return Number.isInteger(n) && n >= 1 ? null : t('repositories.create.errors.retention')
  }

  get hasErrors(): boolean {
    return this.form.invalid || this.quotaError !== null || this.retentionError !== null
  }

  readonly creating = signal(false)

  submit(): void {
    if (this.hasErrors || this.creating()) {
      return
    }
    this.creating.set(true)
    const {
      name,
      format,
      repoType,
      remoteUrl,
      remoteUsername,
      remotePassword,
      quotaMb,
      retentionKeepLastN,
      isPublic,
    } = this.form.getRawValue()
    const quotaBytes = quotaMb.trim() === '' ? null : Math.round(Number(quotaMb) * BYTES_PER_MB)
    const retentionKeepLastNValue =
      retentionKeepLastN.trim() === '' ? null : Number(retentionKeepLastN)

    this.repositoriesService
      .create(name, format, repoType, repoType === 'proxy' ? remoteUrl : null, {
        remoteUsername:
          repoType === 'proxy' && remoteUsername.trim() !== '' ? remoteUsername : null,
        remotePassword:
          repoType === 'proxy' && remotePassword.trim() !== '' ? remotePassword : null,
        groupMembers: repoType === 'group' ? this.groupMembers().map((m) => m.id) : [],
        quotaBytes,
        retentionKeepLastN: retentionKeepLastNValue,
      })
      .subscribe({
        next: (created) => {
          if (!isPublic) {
            this.creating.set(false)
            this.created.emit()
            this.toastService.success(t('repositories.create.created', { name }))
            return
          }
          // Creation has no visibility field: making it public is a second call, made only when
          // ticked.
          this.repositoriesService.setVisibility(created.id, true).subscribe({
            next: () => {
              this.creating.set(false)
              this.created.emit()
              this.toastService.success(t('repositories.create.created', { name }))
            },
            error: (err) => {
              this.creating.set(false)
              // The repository exists; only the visibility call failed.
              this.created.emit()
              this.toastService.error(
                rejectionMessage(err) ?? t('repositories.create.errors.publicFailed'),
              )
            },
          })
        },
        error: (err) => {
          this.creating.set(false)
          this.toastService.error(
            rejectionMessage(err) ?? t('repositories.create.errors.createFailed'),
          )
        },
      })
  }
}
