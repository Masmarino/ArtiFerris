import { TestBed } from '@angular/core/testing'
import { BehaviorSubject, of } from 'rxjs'
import { signal } from '@angular/core'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { ActivatedRoute, Router, convertToParamMap, provideRouter } from '@angular/router'
import { By } from '@angular/platform-browser'
import { Table } from '@masmarino/gabarit/table'
import { RepositoryDetail } from './repository-detail'
import { UsageInstructions } from '../usage-instructions/usage-instructions'
import { PermissionRoleEditor } from '../permission-role-editor/permission-role-editor'
import { PackageTree } from '../package-tree/package-tree'
import { repositoryProviders } from '../infrastructure/repository.providers'
import { ConfirmService } from '../../shared/confirm.service'
import { MeService } from '../../shell/application/me.service'
import { PageTitleService } from '../../shell/page-title.service'
import type { UserLookup } from '../domain/permission.entity'

describe('RepositoryDetail', () => {
  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [{ provide: MeService, useValue: { isSuperAdmin: signal(false) } }],
    })
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('adds a group member and reloads the repository', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'group-1' }) },
            paramMap: of(convertToParamMap({ id: 'group-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/group-1').flush({
      id: 'group-1',
      name: 'group-repo',
      format: 'npm',
      repo_type: 'group',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })

    fixture.componentInstance.newMemberId.set('member-1')
    fixture.componentInstance.addMember()

    const addReq = httpMock.expectOne('/api/repositories/group-1/group-members')
    expect(addReq.request.body).toEqual({ member_repository_id: 'member-1', position: 0 })
    addReq.flush(null)

    httpMock.expectOne('/api/repositories/group-1').flush({
      id: 'group-1',
      name: 'group-repo',
      format: 'npm',
      repo_type: 'group',
      remote_url: null,
      group_members: ['member-1'],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    // A non-empty group_members triggers a lookup of member names.
    httpMock.expectOne('/api/repositories').flush([{ id: 'member-1', name: 'member-repo' }])

    expect(fixture.componentInstance.repository()?.group_members).toEqual(['member-1'])
  })

  it('ignores a second addMember call while the first is still in flight', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'group-1' }) },
            paramMap: of(convertToParamMap({ id: 'group-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/group-1').flush({
      id: 'group-1',
      name: 'group-repo',
      format: 'npm',
      repo_type: 'group',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories/group-1/permissions').flush([])

    fixture.componentInstance.newMemberId.set('member-1')
    fixture.componentInstance.addMember()
    fixture.componentInstance.addMember()

    httpMock.expectOne('/api/repositories/group-1/group-members').flush(null)
    httpMock.expectOne('/api/repositories/group-1').flush({
      id: 'group-1',
      name: 'group-repo',
      format: 'npm',
      repo_type: 'group',
      remote_url: null,
      group_members: ['member-1'],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories/group-1/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([{ id: 'member-1', name: 'member-repo' }])

    httpMock.verify()
  })

  it('shows group member names instead of raw ids, falling back to the id when unknown', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'group-1' }) },
            paramMap: of(convertToParamMap({ id: 'group-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/group-1').flush({
      id: 'group-1',
      name: 'group-repo',
      format: 'npm',
      repo_type: 'group',
      remote_url: null,
      group_members: ['member-1', 'ghost-id'],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories').flush([{ id: 'member-1', name: 'member-repo' }])
    fixture.detectChanges()

    const text = fixture.nativeElement.textContent as string
    expect(text).toContain('member-repo')
    // No name resolved: falls back to the raw id.
    expect(text).toContain('ghost-id')
  })

  it('renames the repository and reloads it', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'old-name',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    fixture.detectChanges()

    expect(fixture.debugElement.query(By.directive(UsageInstructions))).toBeTruthy()
    const packageTree = fixture.debugElement.query(By.directive(PackageTree))
    expect(packageTree).toBeTruthy()
    expect(packageTree.componentInstance.repositoryId()).toBe('repo-1')
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({ format: 'npm', packages: [] })

    fixture.componentInstance.newName.set('new-name')
    fixture.componentInstance.rename()

    const renameReq = httpMock.expectOne('/api/repositories/repo-1')
    expect(renameReq.request.method).toBe('PATCH')
    expect(renameReq.request.body).toEqual({ name: 'new-name' })
    renameReq.flush(null)

    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'new-name',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })

    expect(fixture.componentInstance.repository()?.name).toBe('new-name')
  })

  it('removes a group member and reloads the repository when a member row is clicked', async () => {
    const ask = vi.fn().mockResolvedValue(true)
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        { provide: ConfirmService, useValue: { ask } },
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'group-1' }) },
            paramMap: of(convertToParamMap({ id: 'group-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/group-1').flush({
      id: 'group-1',
      name: 'group-repo',
      format: 'npm',
      repo_type: 'group',
      remote_url: null,
      group_members: ['member-1'],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    // A non-empty group_members triggers a lookup of member names.
    httpMock.expectOne('/api/repositories').flush([{ id: 'member-1', name: 'member-repo' }])
    fixture.detectChanges()

    // Simulates the table's (rowClick) output, to exercise the removeMember binding.
    const tableDebugElement = fixture.debugElement.query(By.directive(Table))
    tableDebugElement.triggerEventHandler('rowClick', { id: 'member-1' })
    await fixture.whenStable()

    expect(ask).toHaveBeenCalledWith(
      expect.objectContaining({
        heading: 'Retirer du groupe',
        message: 'Retirer "member-repo" du groupe ?',
        danger: true,
      }),
    )
    const removeReq = httpMock.expectOne('/api/repositories/group-1/group-members/member-1')
    expect(removeReq.request.method).toBe('DELETE')
    removeReq.flush(null)

    // Removing a member must reload the repository, i.e. fire a second GET.
    httpMock.expectOne('/api/repositories/group-1').flush({
      id: 'group-1',
      name: 'group-repo',
      format: 'npm',
      repo_type: 'group',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })

    expect(fixture.componentInstance.repository()?.group_members).toEqual([])
  })

  it('does not remove a group member when the confirmation is cancelled', async () => {
    const ask = vi.fn().mockResolvedValue(false)
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        { provide: ConfirmService, useValue: { ask } },
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'group-1' }) },
            paramMap: of(convertToParamMap({ id: 'group-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/group-1').flush({
      id: 'group-1',
      name: 'group-repo',
      format: 'npm',
      repo_type: 'group',
      remote_url: null,
      group_members: ['member-1'],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories').flush([{ id: 'member-1', name: 'member-repo' }])
    fixture.detectChanges()

    const tableDebugElement = fixture.debugElement.query(By.directive(Table))
    tableDebugElement.triggerEventHandler('rowClick', { id: 'member-1' })
    await fixture.whenStable()

    expect(ask).toHaveBeenCalled()
    httpMock.expectNone('/api/repositories/group-1/group-members/member-1')
  })

  it('deletes the repository and navigates to the list when the user confirms', async () => {
    const ask = vi.fn().mockResolvedValue(true)
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        { provide: ConfirmService, useValue: { ask } },
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigate')

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'old-name',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })

    await fixture.componentInstance.deleteRepository()

    expect(ask).toHaveBeenCalledWith(
      expect.objectContaining({ heading: 'Supprimer le dépôt', typeToConfirm: 'old-name' }),
    )

    const deleteReq = httpMock.expectOne('/api/repositories/repo-1')
    expect(deleteReq.request.method).toBe('DELETE')
    deleteReq.flush(null)

    expect(router.navigate).toHaveBeenCalledWith(['/repositories'])
  })

  it('exposes searchUsers wired to the username-search endpoint, for the grant autocomplete', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories/repo-1/permissions').flush([])

    let results: UserLookup[] | undefined
    fixture.componentInstance.searchUsers('flo').subscribe((r) => (results = r))
    const searchReq = httpMock.expectOne(
      (r) => r.url === '/api/users/search' && r.params.get('q') === 'flo',
    )
    searchReq.flush([{ id: 'user-2', username: 'florian' }])

    expect(results).toEqual([{ id: 'user-2', username: 'florian' }])
  })

  it('grants a permission to the user selected in the autocomplete, then reloads', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories/repo-1/permissions').flush([])

    fixture.componentInstance.grantUser.set({ id: 'user-2', username: 'florian' })
    fixture.componentInstance.grantRole.set('write')
    fixture.componentInstance.grantPermission()

    httpMock.expectNone((r) => r.url === '/api/users/lookup')
    const grantReq = httpMock.expectOne('/api/repositories/repo-1/permissions/user-2')
    expect(grantReq.request.method).toBe('PUT')
    expect(grantReq.request.body).toEqual({ role: 'write' })
    grantReq.flush(null)

    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    httpMock
      .expectOne('/api/repositories/repo-1/permissions')
      .flush([{ user_id: 'user-2', username: 'florian', role: 'write' }])

    expect(fixture.componentInstance.grantUser()).toBeNull()
    expect(fixture.componentInstance.permissions()).toEqual([
      { user_id: 'user-2', username: 'florian', role: 'write' },
    ])
  })

  it('opens the role editor when a permission row is clicked, and revokes on confirmation', async () => {
    const ask = vi.fn().mockResolvedValue(true)
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        { provide: ConfirmService, useValue: { ask } },
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'old-name',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    httpMock
      .expectOne('/api/repositories/repo-1/permissions')
      .flush([{ user_id: 'user-2', username: 'florian', role: 'write' }])
    fixture.detectChanges()

    const tableDebugElement = fixture.debugElement.query(By.directive(Table))
    tableDebugElement.triggerEventHandler('rowClick', {
      user_id: 'user-2',
      username: 'florian',
      role: 'write',
    })
    fixture.detectChanges()

    expect(fixture.componentInstance.editingPermission()).toEqual({
      user_id: 'user-2',
      username: 'florian',
      role: 'write',
    })

    const editorDebugElement = fixture.debugElement.query(By.directive(PermissionRoleEditor))
    editorDebugElement.triggerEventHandler('revoked', undefined)

    expect(ask).not.toHaveBeenCalled() // confirmation now happens inside PermissionRoleEditor, not repository-detail
    const revokeReq = httpMock.expectOne('/api/repositories/repo-1/permissions/user-2')
    expect(revokeReq.request.method).toBe('DELETE')
    revokeReq.flush(null)

    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'old-name',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories/repo-1/permissions').flush([])

    expect(fixture.componentInstance.permissions()).toEqual([])
  })

  it('changes a permission role via the editor and reloads', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'old-name',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    httpMock
      .expectOne('/api/repositories/repo-1/permissions')
      .flush([{ user_id: 'user-2', username: 'florian', role: 'read' }])
    fixture.detectChanges()

    const tableDebugElement = fixture.debugElement.query(By.directive(Table))
    tableDebugElement.triggerEventHandler('rowClick', {
      user_id: 'user-2',
      username: 'florian',
      role: 'read',
    })
    fixture.detectChanges()

    const editorDebugElement = fixture.debugElement.query(By.directive(PermissionRoleEditor))
    editorDebugElement.triggerEventHandler('roleChanged', 'admin')

    const grantReq = httpMock.expectOne('/api/repositories/repo-1/permissions/user-2')
    expect(grantReq.request.method).toBe('PUT')
    expect(grantReq.request.body).toEqual({ role: 'admin' })
    grantReq.flush(null)

    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'old-name',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    httpMock
      .expectOne('/api/repositories/repo-1/permissions')
      .flush([{ user_id: 'user-2', username: 'florian', role: 'admin' }])

    expect(fixture.componentInstance.permissions()).toEqual([
      { user_id: 'user-2', username: 'florian', role: 'admin' },
    ])
  })

  it('does not delete the repository when the confirmation is cancelled', async () => {
    const ask = vi.fn().mockResolvedValue(false)
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        { provide: ConfirmService, useValue: { ask } },
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigate')

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'old-name',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })

    await fixture.componentInstance.deleteRepository()

    expect(ask).toHaveBeenCalled()
    httpMock.expectNone('/api/repositories/repo-1')
    expect(router.navigate).not.toHaveBeenCalled()
  })

  it('sets the shared page title to the repository name once it loads', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)
    const pageTitle = TestBed.inject(PageTitleService)

    fixture.detectChanges()
    expect(pageTitle.title()).toBe('')

    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    fixture.detectChanges()

    expect(pageTitle.title()).toBe('my-repo')
  })

  it('hides the admin-only tabs entirely for a read-only viewer', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'read',
    })
    httpMock
      .expectOne('/api/repositories/repo-1/permissions')
      .flush([{ user_id: 'user-1', username: 'someone', role: 'read' }])
    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({ format: 'npm', packages: [] })

    const text = fixture.nativeElement.textContent as string
    expect(text).not.toContain('Renommer')
    expect(text).not.toContain('Supprimer le dépôt')
    expect(text).not.toContain('Accorder')

    // Not rendered at all for a read-only viewer, rather than shown read-only.
    expect(text).not.toContain("Droits d'accès")
    expect(text).not.toContain('Paramètres')
    expect(text).not.toContain('someone')
    expect(fixture.debugElement.query(By.directive(Table))).toBeNull()
  })

  it('shows the public overview instead of an error for a viewer with only implicit public read', () => {
    // An implicit read on a public repository gets 403/404 on the permissions list, with my_role
    // 'read' like an explicit grant:
    // the page must not be blocked.
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      is_public: true,
      my_role: 'read',
      owner_name: 'alice',
      owner_is_personal: true,
      public_path: '/@alice/my-repo',
    })
    httpMock
      .expectOne('/api/repositories/repo-1/permissions')
      .flush({}, { status: 404, statusText: 'Not Found' })
    fixture.detectChanges()

    expect(fixture.componentInstance.loadError()).toBe(false)
    expect(fixture.nativeElement.textContent).not.toContain('Échec du chargement')
    expect(fixture.nativeElement.textContent).toContain('/@alice/my-repo')
  })

  it('shows rename, delete, and permission-management controls for a repository admin', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories/repo-1/permissions').flush([])
    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({ format: 'npm', packages: [] })

    const text = fixture.nativeElement.textContent as string
    expect(text).toContain('Renommer')
    expect(text).toContain('Supprimer le dépôt')
    expect(text).toContain('Accorder')
  })

  it('loads the current quota in MB and saves an updated value converted to bytes', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: 5 * 1024 * 1024,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories/repo-1/permissions').flush([])
    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({ format: 'npm', packages: [] })

    expect(fixture.componentInstance.quotaMb()).toBe('5')

    fixture.componentInstance.setQuotaMb('10')
    fixture.componentInstance.saveQuota()

    const putReq = httpMock.expectOne('/api/repositories/repo-1/quota')
    expect(putReq.request.method).toBe('PUT')
    expect(putReq.request.body).toEqual({ quota_bytes: 10 * 1024 * 1024 })
    putReq.flush(null)

    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: 10 * 1024 * 1024,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories/repo-1/permissions').flush([])

    expect(fixture.componentInstance.quotaSaved()).toBe(true)
  })

  it('clears the quota back to unlimited when the field is left empty', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: 5 * 1024 * 1024,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories/repo-1/permissions').flush([])
    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({ format: 'npm', packages: [] })

    fixture.componentInstance.setQuotaMb('')
    fixture.componentInstance.saveQuota()

    const putReq = httpMock.expectOne('/api/repositories/repo-1/quota')
    expect(putReq.request.body).toEqual({ quota_bytes: null })
    putReq.flush(null)

    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories/repo-1/permissions').flush([])
  })

  it('rejects a negative quota without sending a request', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories/repo-1/permissions').flush([])
    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({ format: 'npm', packages: [] })

    fixture.componentInstance.setQuotaMb('-5')
    fixture.componentInstance.saveQuota()

    httpMock.expectNone('/api/repositories/repo-1/quota')
    expect(fixture.componentInstance.quotaError()).toContain('positif')
  })

  it('hides the quota and retention settings entirely for a non-admin viewer', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: 2 * 1024 * 1024,
      retention_keep_last_n: null,
      my_role: 'read',
    })
    httpMock
      .expectOne('/api/repositories/repo-1/permissions')
      .flush([{ user_id: 'u1', username: 'someone', role: 'read' }])
    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({ format: 'npm', packages: [] })

    const text = fixture.nativeElement.textContent as string
    expect(text).not.toContain('2.0 Mo')
    expect(text).not.toContain('Quota de stockage')
    expect(text).not.toContain('Enregistrer')
  })

  it('treats a repository view without quota or retention fields as unlimited and disabled', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories/repo-1/permissions').flush([])
    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({ format: 'npm', packages: [] })

    const text = fixture.nativeElement.textContent as string
    expect(text).toContain('Actuel : illimité')
    expect(text).toContain('Actuel : désactivée')
    expect(text).not.toContain('undefined')
    expect(text).not.toContain('NaN')
    expect(fixture.componentInstance.quotaMb()).toBe('')
    expect(fixture.componentInstance.retentionKeepLastN()).toBe('')
  })

  it('loads the current retention policy and saves an updated value', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: 3,
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories/repo-1/permissions').flush([])
    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({ format: 'npm', packages: [] })

    expect(fixture.componentInstance.retentionKeepLastN()).toBe('3')

    fixture.componentInstance.setRetentionKeepLastN('10')
    fixture.componentInstance.saveRetentionPolicy()

    const putReq = httpMock.expectOne('/api/repositories/repo-1/retention')
    expect(putReq.request.method).toBe('PUT')
    expect(putReq.request.body).toEqual({ keep_last_n_versions: 10 })
    putReq.flush(null)

    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: 10,
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories/repo-1/permissions').flush([])

    expect(fixture.componentInstance.retentionSaved()).toBe(true)
  })

  it('disables the retention policy when the field is left empty', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: 3,
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories/repo-1/permissions').flush([])
    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({ format: 'npm', packages: [] })

    fixture.componentInstance.setRetentionKeepLastN('')
    fixture.componentInstance.saveRetentionPolicy()

    const putReq = httpMock.expectOne('/api/repositories/repo-1/retention')
    expect(putReq.request.body).toEqual({ keep_last_n_versions: null })
    putReq.flush(null)

    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories/repo-1/permissions').flush([])
  })

  it('rejects a retention value below 1 without sending a request', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...repositoryProviders,
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
            paramMap: of(convertToParamMap({ id: 'repo-1' })),
          },
        },
      ],
    })
    const fixture = TestBed.createComponent(RepositoryDetail)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1').flush({
      id: 'repo-1',
      name: 'my-repo',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
    })
    httpMock.expectOne('/api/repositories/repo-1/permissions').flush([])
    fixture.detectChanges()
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({ format: 'npm', packages: [] })

    fixture.componentInstance.setRetentionKeepLastN('0')
    fixture.componentInstance.saveRetentionPolicy()

    httpMock.expectNone('/api/repositories/repo-1/retention')
    expect(fixture.componentInstance.retentionError()).toContain('entier positif')
  })

  describe('visibility', () => {
    const REPO = {
      id: 'repo-1',
      name: 'my-project',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      is_public: false,
      my_role: 'admin',
      owner_is_personal: true,
    }

    function render(repo: object, answer: boolean, superAdmin = false) {
      const ask = vi.fn().mockResolvedValue(answer)
      TestBed.configureTestingModule({
        providers: [
          provideHttpClient(),
          provideHttpClientTesting(),
          ...repositoryProviders,
          { provide: ConfirmService, useValue: { ask } },
          { provide: MeService, useValue: { isSuperAdmin: signal(superAdmin) } },
          provideRouter([]),
          {
            provide: ActivatedRoute,
            useValue: {
              snapshot: { paramMap: convertToParamMap({ id: 'repo-1' }) },
              paramMap: of(convertToParamMap({ id: 'repo-1' })),
            },
          },
        ],
      })
      const fixture = TestBed.createComponent(RepositoryDetail)
      const httpMock = TestBed.inject(HttpTestingController)
      fixture.detectChanges()
      httpMock.expectOne('/api/repositories/repo-1').flush(repo)
      httpMock.expectOne('/api/repositories/repo-1/permissions').flush([])
      return { fixture, httpMock, ask }
    }

    it('lets the owner of a personal project change its visibility', () => {
      const { fixture } = render(REPO, true)

      expect(fixture.componentInstance.canChangeVisibility()).toBe(true)
    })

    it('offers it to any admin of the repository, as the server does, organization repository included', () => {
      const orgRepo = { ...REPO, owner_is_personal: false }

      expect(render(orgRepo, true).fixture.componentInstance.canChangeVisibility()).toBe(true)
    })

    it('does not offer it to someone who is not an admin of the repository', () => {
      const { fixture } = render({ ...REPO, my_role: 'write' }, true)

      expect(fixture.componentInstance.canChangeVisibility()).toBe(false)
    })

    it('offers it to a super-admin', () => {
      const { fixture } = render({ ...REPO, my_role: null }, true, true)

      expect(fixture.componentInstance.canChangeVisibility()).toBe(true)
    })

    it('never offers it for a proxy or a group', () => {
      const { fixture } = render({ ...REPO, repo_type: 'proxy' }, true, true)

      expect(fixture.componentInstance.canChangeVisibility()).toBe(false)
    })

    it('asks for a danger confirmation, then publishes the repository', async () => {
      const { fixture, httpMock, ask } = render(REPO, true)

      const done = fixture.componentInstance.changeVisibility()
      await Promise.resolve()

      expect(ask).toHaveBeenCalledWith(
        expect.objectContaining({ heading: 'Rendre le dépôt public', danger: true }),
      )
      const req = httpMock.expectOne('/api/repositories/repo-1/visibility')
      expect(req.request.method).toBe('PUT')
      expect(req.request.body).toEqual({ is_public: true })
      req.flush(null)
      await done
      httpMock.expectOne('/api/repositories/repo-1').flush({ ...REPO, is_public: true })
      httpMock.expectOne('/api/repositories/repo-1/permissions').flush([])
      expect(fixture.componentInstance.repository()?.is_public).toBe(true)
    })

    it('makes a public repository private without a danger confirmation', async () => {
      const { fixture, httpMock, ask } = render({ ...REPO, is_public: true }, true)

      void fixture.componentInstance.changeVisibility()
      await Promise.resolve()

      expect(ask).toHaveBeenCalledWith(
        expect.objectContaining({ heading: 'Rendre le dépôt privé' }),
      )
      expect(ask.mock.calls[0][0].danger).toBeUndefined()
      expect(httpMock.expectOne('/api/repositories/repo-1/visibility').request.body).toEqual({
        is_public: false,
      })
    })

    it('changes nothing when the confirmation is declined', async () => {
      const { fixture, httpMock } = render(REPO, false)

      await fixture.componentInstance.changeVisibility()

      httpMock.expectNone('/api/repositories/repo-1/visibility')
    })
  })

  describe('navigating from one repository to another', () => {
    const repo = (id: string, name: string, myRole = 'admin') => ({
      id,
      name,
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      quota_bytes: 5 * 1024 * 1024,
      retention_keep_last_n: 3,
      is_public: false,
      my_role: myRole,
    })

    function setupNavigable() {
      const paramMap$ = new BehaviorSubject(convertToParamMap({ id: 'repo-a' }))
      TestBed.configureTestingModule({
        providers: [
          provideHttpClient(),
          provideHttpClientTesting(),
          ...repositoryProviders,
          provideRouter([]),
          {
            provide: ActivatedRoute,
            useValue: { snapshot: { paramMap: paramMap$.value }, paramMap: paramMap$ },
          },
        ],
      })
      const fixture = TestBed.createComponent(RepositoryDetail)
      const httpMock = TestBed.inject(HttpTestingController)
      fixture.detectChanges()
      httpMock.expectOne('/api/repositories/repo-a').flush(repo('repo-a', 'alpha'))
      httpMock
        .expectOne('/api/repositories/repo-a/permissions')
        .flush([{ user_id: 'u1', username: 'alice', role: 'write' }])
      fixture.detectChanges()
      const goTo = (id: string) => {
        paramMap$.next(convertToParamMap({ id }))
        fixture.detectChanges()
      }
      return { fixture, httpMock, goTo }
    }

    it("drops repository A's data while repository B loads", () => {
      const { fixture, httpMock, goTo } = setupNavigable()
      const page = fixture.componentInstance
      page.openRoleEditor({ user_id: 'u1', username: 'alice', role: 'write' })
      expect(page.repository()?.name).toBe('alpha')
      expect(page.quotaMb()).toBe('5')

      goTo('repo-b')

      expect(page.repository()).toBeNull()
      expect(page.permissions()).toEqual([])
      expect(page.editingPermission()).toBeNull()
      expect(page.newName()).toBe('')
      expect(page.quotaMb()).toBe('')
      expect(page.retentionKeepLastN()).toBe('')
      expect(fixture.nativeElement.textContent).not.toContain('alpha')
      httpMock.match(() => true)
    })

    it("cannot rename, delete or change repository A through B's page", () => {
      const { fixture, httpMock, goTo } = setupNavigable()
      const page = fixture.componentInstance
      page.newName.set('renamed')

      goTo('repo-b')
      page.rename()
      page.saveQuota()
      page.saveRetentionPolicy()
      page.changeRole('read')
      page.revokeFromEditor()
      void page.deleteRepository()
      void page.changeVisibility()

      httpMock.expectNone((req) => req.method !== 'GET')
      httpMock.match(() => true)
    })

    it("does not show A's quota-saved notice on B when A's write finishes late", () => {
      const { fixture, httpMock, goTo } = setupNavigable()
      const page = fixture.componentInstance
      page.setQuotaMb('9')
      page.saveQuota()
      const quotaRequest = httpMock.expectOne('/api/repositories/repo-a/quota')

      goTo('repo-b')
      httpMock.expectOne('/api/repositories/repo-b').flush(repo('repo-b', 'beta'))
      httpMock.expectOne('/api/repositories/repo-b/permissions').flush([])
      quotaRequest.flush(null)

      expect(page.quotaSaved()).toBe(false)
      httpMock.expectNone('/api/repositories/repo-a')
      httpMock.expectNone('/api/repositories/repo-a/permissions')
      expect(page.repository()?.name).toBe('beta')
    })

    it('shows the load error when the permissions request fails', () => {
      const { fixture, httpMock, goTo } = setupNavigable()

      goTo('repo-b')
      httpMock.expectOne('/api/repositories/repo-b').flush(repo('repo-b', 'beta'))
      httpMock
        .expectOne('/api/repositories/repo-b/permissions')
        .flush({}, { status: 500, statusText: 'Error' })
      fixture.detectChanges()

      expect(fixture.componentInstance.loadError()).toBe(true)
      expect(fixture.nativeElement.querySelector('[role="alert"]')).toBeTruthy()
    })
  })
})
