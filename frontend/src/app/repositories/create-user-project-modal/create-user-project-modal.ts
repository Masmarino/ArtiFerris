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
import { Button, Checkbox, GbtInput, Modal, Select, type SelectOption } from '@masmarino/gabarit'
import { PersonalRepositoryService } from '../application/personal-repository.service'
import { RepositoriesService } from '../application/repositories.service'
import { RepositoryFormat, RepositoryType } from '../domain/repository.entity'
import { ToastService } from '../../shared/toast.service'
import { rejectionMessage } from '../../shared/api-error'

const FORMAT_OPTIONS: SelectOption<RepositoryFormat>[] = [
  { value: 'npm', label: 'npm' },
  { value: 'docker', label: 'docker' },
]

// No route can ever set remote_url on a personal project after creation, so proxy
// is excluded here — unlike the org-level modal, which does support it.
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

  // Zoneless only re-renders on signal changes, so this can't just read the FormControl directly.
  private readonly repoType = toSignal(this.form.controls.repoType.valueChanges, {
    initialValue: this.form.controls.repoType.value,
  })
  // No proxy option here (see REPO_TYPE_OPTIONS above), so group is the only type the backend
  // rejects a public toggle for.
  readonly canBePublic = computed(() => this.repoType() !== 'group')

  readonly creating = signal(false)

  constructor() {
    // Drop a stale checked state so it can't survive a repo-type change while the checkbox row
    // is hidden and still get submitted.
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
        // The creation endpoint has no visibility field — making it public is a second,
        // separate call, only ever fired when the checkbox was ticked.
        this.repositoriesService.setVisibility(created.id, true).subscribe({
          next: () => {
            this.creating.set(false)
            this.created.emit()
            this.toastService.success(t('repositories.createProject.created', { name }))
          },
          error: (err) => {
            this.creating.set(false)
            // The project itself was created — only the visibility follow-up failed.
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
