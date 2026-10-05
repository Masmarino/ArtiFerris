import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  output,
  signal,
} from '@angular/core'
import { toSignal } from '@angular/core/rxjs-interop'
import { FormControl, FormGroup, ReactiveFormsModule, Validators } from '@angular/forms'
import { Button } from '@masmarino/gabarit/button'
import { Checkbox } from '@masmarino/gabarit/checkbox'
import { GbtInput } from '@masmarino/gabarit/input'
import { Modal } from '@masmarino/gabarit/modal'
import { Select, type SelectOption } from '@masmarino/gabarit/select'
import { PersonalRepositoryService } from '../application/personal-repository.service'
import { RepositoriesService } from '../application/repositories.service'
import { RepositoryFormat, RepositoryType } from '../domain/repository.entity'
import { ToastService } from '../../shared/toast.service'
import { rejectionMessage } from '../../shared/api-error'

const FORMAT_OPTIONS: SelectOption<RepositoryFormat>[] = [
  { value: 'npm', label: 'npm' },
  { value: 'docker', label: 'docker' },
]

// No route sets remote_url on a personal project, so there is no proxy option here.
const REPO_TYPE_OPTIONS: SelectOption<RepositoryType>[] = [
  { value: 'hosted', label: 'hosted' },
  { value: 'group', label: 'group' },
]

@Component({
  selector: 'app-create-user-project-modal',
  standalone: true,
  imports: [TranslocoPipe, ReactiveFormsModule, Modal, GbtInput, Select, Checkbox, Button],
  templateUrl: './create-user-project-modal.html',
  styleUrl: './create-user-project-modal.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class CreateUserProjectModal {
  private readonly personalRepositoryService = inject(PersonalRepositoryService)
  private readonly repositoriesService = inject(RepositoriesService)
  private readonly toastService = inject(ToastService)

  readonly created = output<void>()
  readonly cancelled = output<void>()

  readonly formatOptions = FORMAT_OPTIONS
  readonly repoTypeOptions = REPO_TYPE_OPTIONS

  readonly form = new FormGroup({
    name: new FormControl('', { nonNullable: true, validators: [Validators.required] }),
    format: new FormControl<RepositoryFormat>('npm', { nonNullable: true }),
    repoType: new FormControl<RepositoryType>('hosted', { nonNullable: true }),
    isPublic: new FormControl(false, { nonNullable: true }),
  })

  // Zoneless: only signals trigger a re-render, so the FormControl can't be read directly.
  private readonly repoType = toSignal(this.form.controls.repoType.valueChanges, {
    initialValue: this.form.controls.repoType.value,
  })
  // No proxy here, so group is the only type the backend refuses a public toggle for.
  readonly canBePublic = computed(() => this.repoType() !== 'group')

  readonly creating = signal(false)

  constructor() {
    // Drop a stale checked state so it is not submitted while the checkbox is hidden.
    effect(() => {
      if (!this.canBePublic()) {
        this.form.controls.isPublic.setValue(false)
      }
    })
  }

  submit(): void {
    if (this.form.invalid || this.creating()) {
      return
    }
    this.creating.set(true)
    const { name, format, repoType, isPublic } = this.form.getRawValue()

    this.personalRepositoryService.createProject(name, format, repoType).subscribe({
      next: (created) => {
        if (!isPublic) {
          this.creating.set(false)
          this.created.emit()
          this.toastService.success(t('repositories.createProject.created', { name }))
          return
        }
        // Creation has no visibility field: making it public is a second call, made only when
        // ticked.
        this.repositoriesService.setVisibility(created.id, true).subscribe({
          next: () => {
            this.creating.set(false)
            this.created.emit()
            this.toastService.success(t('repositories.createProject.created', { name }))
          },
          error: (err) => {
            this.creating.set(false)
            // The project exists; only the visibility call failed.
            this.created.emit()
            this.toastService.error(
              rejectionMessage(err) ?? t('repositories.createProject.errors.publicFailed'),
            )
          },
        })
      },
      error: (err) => {
        this.creating.set(false)
        this.toastService.error(
          rejectionMessage(err) ?? t('repositories.createProject.errors.createFailed'),
        )
      },
    })
  }
}
