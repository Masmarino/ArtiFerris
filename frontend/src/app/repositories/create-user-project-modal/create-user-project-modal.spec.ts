import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { CreateUserProjectModal } from './create-user-project-modal'
import { repositoryProviders } from '../infrastructure/repository.providers'
import { ToastService } from '../../shared/toast.service'

function render() {
  TestBed.configureTestingModule({
    providers: [provideHttpClient(), provideHttpClientTesting(), ...repositoryProviders],
  })
  const fixture = TestBed.createComponent(CreateUserProjectModal)
  const httpMock = TestBed.inject(HttpTestingController)
  fixture.detectChanges()
  return { fixture, httpMock }
}

const CREATED_PROJECT = {
  id: 'repo-1',
  name: 'my-project',
  format: 'npm',
  repo_type: 'hosted',
  remote_url: null,
  remote_credentials_set: false,
  group_members: [],
  quota_bytes: null,
  retention_keep_last_n: null,
  my_role: 'admin',
  organization_id: 'org-1',
  owner_name: 'florian',
  owner_is_personal: true,
}

describe('CreateUserProjectModal', () => {
  it('posts name/format/repo_type via PersonalRepositoryService.createProject()', () => {
    const { fixture, httpMock } = render()
    fixture.componentInstance.form.controls.name.setValue('my-project')
    fixture.componentInstance.form.controls.format.setValue('docker')
    fixture.componentInstance.form.controls.repoType.setValue('group')

    fixture.componentInstance.submit()

    const req = httpMock.expectOne('/api/me/repository/projects')
    expect(req.request.method).toBe('POST')
    expect(req.request.body).toEqual({ name: 'my-project', format: 'docker', repo_type: 'group' })
    req.flush(CREATED_PROJECT)
  })

  it('does not call setVisibility when the visibility checkbox is left unchecked', () => {
    const { fixture, httpMock } = render()
    fixture.componentInstance.form.controls.name.setValue('my-project')

    fixture.componentInstance.submit()

    httpMock.expectOne('/api/me/repository/projects').flush(CREATED_PROJECT)

    httpMock.expectNone('/api/repositories/repo-1/visibility')
  })

  it('calls setVisibility with the newly created id before emitting created when the checkbox is checked', () => {
    const { fixture, httpMock } = render()
    fixture.componentInstance.form.controls.name.setValue('my-project')
    fixture.componentInstance.form.controls.isPublic.setValue(true)
    let created = false
    fixture.componentInstance.created.subscribe(() => (created = true))

    fixture.componentInstance.submit()

    httpMock.expectOne('/api/me/repository/projects').flush(CREATED_PROJECT)
    expect(created).toBe(false)

    const visibilityReq = httpMock.expectOne('/api/repositories/repo-1/visibility')
    expect(visibilityReq.request.method).toBe('PUT')
    expect(visibilityReq.request.body).toEqual({ is_public: true })
    expect(created).toBe(false)
    visibilityReq.flush(null)

    expect(created).toBe(true)
  })

  it('emits created right after createProject() when the checkbox is unchecked, with no visibility call in between', () => {
    const { fixture, httpMock } = render()
    fixture.componentInstance.form.controls.name.setValue('my-project')
    let created = false
    fixture.componentInstance.created.subscribe(() => (created = true))

    fixture.componentInstance.submit()

    httpMock.expectOne('/api/me/repository/projects').flush(CREATED_PROJECT)

    expect(created).toBe(true)
  })

  it('shows a success toast naming the project once created', () => {
    const { fixture, httpMock } = render()
    fixture.componentInstance.form.controls.name.setValue('my-project')

    fixture.componentInstance.submit()

    httpMock.expectOne('/api/me/repository/projects').flush(CREATED_PROJECT)

    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
      variant: 'success',
      message: 'Projet « my-project » créé.',
    })
  })

  it('shows an error toast with the server message when creation fails', () => {
    const { fixture, httpMock } = render()
    fixture.componentInstance.form.controls.name.setValue('my-project')

    fixture.componentInstance.submit()

    httpMock
      .expectOne('/api/me/repository/projects')
      .flush(
        { error: 'un projet nommé « my-project » existe déjà' },
        { status: 409, statusText: 'Conflict' },
      )

    expect(fixture.componentInstance.creating()).toBe(false)
    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
      variant: 'error',
      message: 'un projet nommé « my-project » existe déjà',
    })
  })

  it('still emits created and shows an error toast when the visibility follow-up fails', () => {
    const { fixture, httpMock } = render()
    fixture.componentInstance.form.controls.name.setValue('my-project')
    fixture.componentInstance.form.controls.isPublic.setValue(true)
    let created = false
    fixture.componentInstance.created.subscribe(() => (created = true))

    fixture.componentInstance.submit()

    httpMock.expectOne('/api/me/repository/projects').flush(CREATED_PROJECT)
    httpMock
      .expectOne('/api/repositories/repo-1/visibility')
      .flush({ error: 'échec' }, { status: 500, statusText: 'Internal Server Error' })

    expect(created).toBe(true)
    expect(fixture.componentInstance.creating()).toBe(false)
    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({ variant: 'error' })
  })

  it('does not submit an empty name', () => {
    const { fixture, httpMock } = render()

    fixture.componentInstance.submit()

    httpMock.expectNone((r) => r.url === '/api/me/repository/projects')
  })

  describe('the "Rendre public" checkbox', () => {
    it('does not render when repoType is group', () => {
      const { fixture } = render()

      fixture.componentInstance.form.controls.repoType.setValue('group')
      fixture.detectChanges()

      expect(fixture.nativeElement.querySelector('gbt-checkbox')).toBeNull()
    })

    it('renders when repoType is a hosted-capable type', () => {
      const { fixture } = render()

      expect(fixture.nativeElement.querySelector('gbt-checkbox')).not.toBeNull()
    })

    it('clears a checked state when switching away from hosted', () => {
      const { fixture } = render()
      fixture.componentInstance.form.controls.isPublic.setValue(true)

      fixture.componentInstance.form.controls.repoType.setValue('group')
      fixture.detectChanges()

      expect(fixture.componentInstance.form.controls.isPublic.value).toBe(false)
    })
  })
})
