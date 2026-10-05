import { TestBed } from '@angular/core/testing'
import { BehaviorSubject, of } from 'rxjs'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { ActivatedRoute, Router, convertToParamMap, provideRouter } from '@angular/router'
import { By } from '@angular/platform-browser'
import { PermissionRoleEditor } from '../../repositories/permission-role-editor/permission-role-editor'
import { UserDetail } from './user-detail'
import { PageTitleService } from '../../shell/page-title.service'
import { userProviders } from '../infrastructure/user.providers'
import { repositoryProviders } from '../../repositories/infrastructure/repository.providers'
import { MeService } from '../../shell/application/me.service'
import { ToastService } from '../../shared/toast.service'
import { ConfirmService } from '../../shared/confirm.service'

function userFixture(overrides: Record<string, unknown>) {
  return {
    is_super_admin: false,
    organization_id: 'org-acme',
    email: null,
    invitation_pending: false,
    created_at: '2026-08-22T09:00:00Z',
    invitation_expires_at: null,
    mfa_enabled: false,
    ...overrides,
  }
}

describe('UserDetail', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  // Defaults to a super-admin viewer — org-admin restrictions get their own tests below.
  function setup(options?: { isSuperAdmin?: boolean; confirmed?: boolean }) {
    const ask = vi.fn().mockResolvedValue(options?.confirmed ?? true)
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        provideRouter([]),
        ...userProviders,
        ...repositoryProviders,
        {
          provide: ActivatedRoute,
          useValue: {
            snapshot: { paramMap: convertToParamMap({ id: 'user-2' }) },
            paramMap: of(convertToParamMap({ id: 'user-2' })),
          },
        },
        {
          provide: MeService,
          useValue: {
            isSuperAdmin: () => options?.isSuperAdmin ?? true,
            username: () => 'admin',
          },
        },
        { provide: ConfirmService, useValue: { ask } },
      ],
    })
    return {
      ask,
      fixture: TestBed.createComponent(UserDetail),
      httpMock: TestBed.inject(HttpTestingController),
    }
  }

  it('loads the user and their permissions across repositories', () => {
    const { fixture, httpMock } = setup()
    fixture.detectChanges()

    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: false }))
    httpMock
      .expectOne('/api/users/user-2/permissions')
      .flush([
        { repository_id: 'repo-1', repository_name: 'my-repo', format: 'npm', role: 'write' },
      ])
    httpMock.expectOne('/api/repositories').flush([])

    expect(fixture.componentInstance.username()).toBe('florian')
    expect(fixture.componentInstance.permissions()).toEqual([
      { repository_id: 'repo-1', repository_name: 'my-repo', format: 'npm', role: 'write' },
    ])
  })

  it('fetches the single user by id, not the whole user list', () => {
    const { fixture, httpMock } = setup()
    fixture.detectChanges()

    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: false }))
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([])

    httpMock.expectNone('/api/users')
  })

  it('shows a blank state instead of erroring when the user id is unknown', () => {
    const { fixture, httpMock } = setup()
    fixture.detectChanges()

    httpMock
      .expectOne('/api/users/user-2')
      .flush('not found', { status: 404, statusText: 'Not Found' })
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([])

    expect(fixture.componentInstance.user()).toBeNull()
  })

  it('changes a role via the editor and reloads', () => {
    const { fixture, httpMock } = setup()
    fixture.detectChanges()
    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: false }))
    httpMock
      .expectOne('/api/users/user-2/permissions')
      .flush([{ repository_id: 'repo-1', repository_name: 'my-repo', format: 'npm', role: 'read' }])
    httpMock.expectOne('/api/repositories').flush([])
    fixture.detectChanges()

    fixture.componentInstance.openRoleEditor(fixture.componentInstance.permissions()[0])
    fixture.detectChanges()

    const editorDebugElement = fixture.debugElement.query(By.directive(PermissionRoleEditor))
    editorDebugElement.triggerEventHandler('roleChanged', 'write')

    const grantReq = httpMock.expectOne('/api/repositories/repo-1/permissions/user-2')
    expect(grantReq.request.method).toBe('PUT')
    expect(grantReq.request.body).toEqual({ role: 'write' })
    grantReq.flush(null)

    // user list is cached and unchanged by a permission edit, so no re-fetch here
    httpMock
      .expectOne('/api/users/user-2/permissions')
      .flush([
        { repository_id: 'repo-1', repository_name: 'my-repo', format: 'npm', role: 'write' },
      ])

    expect(fixture.componentInstance.permissions()[0].role).toBe('write')
  })

  it('deletes the user and navigates to the users list when confirmed', async () => {
    const { fixture, httpMock, ask } = setup()
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigate')

    fixture.detectChanges()
    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: false }))
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([])

    await fixture.componentInstance.deleteUser()

    expect(ask).toHaveBeenCalledWith(
      expect.objectContaining({ heading: "Supprimer l'utilisateur", typeToConfirm: 'florian' }),
    )
    const deleteReq = httpMock.expectOne('/api/users/user-2')
    expect(deleteReq.request.method).toBe('DELETE')
    deleteReq.flush(null)

    expect(router.navigate).toHaveBeenCalledWith(['/users'])
  })

  it('does not delete the user when the confirmation is cancelled', async () => {
    const { fixture, httpMock, ask } = setup({ confirmed: false })
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigate')

    fixture.detectChanges()
    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: false }))
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([])

    await fixture.componentInstance.deleteUser()

    expect(ask).toHaveBeenCalled()
    httpMock.expectNone('/api/users/user-2')
    expect(router.navigate).not.toHaveBeenCalled()
  })

  it('revokes a permission via the editor for the correct repository, keeping the user id fixed', () => {
    const { fixture, httpMock } = setup()

    fixture.detectChanges()
    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: false }))
    httpMock
      .expectOne('/api/users/user-2/permissions')
      .flush([
        { repository_id: 'repo-9', repository_name: 'other-repo', format: 'npm', role: 'write' },
      ])
    httpMock.expectOne('/api/repositories').flush([])
    fixture.detectChanges()

    fixture.componentInstance.openRoleEditor(fixture.componentInstance.permissions()[0])
    fixture.detectChanges()

    expect(fixture.componentInstance.editingPermission()).toEqual({
      repository_id: 'repo-9',
      repository_name: 'other-repo',
      format: 'npm',
      role: 'write',
    })

    const editorDebugElement = fixture.debugElement.query(By.directive(PermissionRoleEditor))
    editorDebugElement.triggerEventHandler('revoked', undefined)

    const revokeReq = httpMock.expectOne('/api/repositories/repo-9/permissions/user-2')
    expect(revokeReq.request.method).toBe('DELETE')
    revokeReq.flush(null)

    // user list is cached and unchanged by a permission edit, so no re-fetch here
    httpMock.expectOne('/api/users/user-2/permissions').flush([])

    expect(fixture.componentInstance.editingPermission()).toBeNull()
    expect(fixture.componentInstance.permissions()).toEqual([])
  })

  it('promotes a user to super-admin at once, as the list does', async () => {
    const { fixture, httpMock, ask } = setup()
    fixture.detectChanges()
    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: false }))
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([])

    await fixture.componentInstance.setSuperAdmin()

    expect(ask).not.toHaveBeenCalled()

    const req = httpMock.expectOne('/api/users/user-2/super-admin')
    expect(req.request.method).toBe('PUT')
    expect(req.request.body).toEqual({ is_super_admin: true })
    req.flush(null)

    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: true }))
    httpMock.expectOne('/api/users/user-2/permissions').flush([])

    expect(fixture.componentInstance.user()?.is_super_admin).toBe(true)
  })

  it('shows an error when demoting the last super-admin is rejected', async () => {
    const { fixture, httpMock, ask } = setup()
    fixture.detectChanges()
    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: true }))
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([])

    await fixture.componentInstance.setSuperAdmin()

    expect(ask).toHaveBeenCalledWith(
      expect.objectContaining({
        heading: 'Retirer les droits de super-administrateur',
        message: expect.stringContaining('florian ne pourra plus gérer les utilisateurs'),
      }),
    )

    const req = httpMock.expectOne('/api/users/user-2/super-admin')
    req.flush({ error: 'conflict' }, { status: 409, statusText: 'Conflict' })
    fixture.detectChanges()

    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
      variant: 'error',
      message: 'Impossible de rétrograder le dernier super-administrateur.',
    })
  })

  it('shows a generic error when a non-conflict failure occurs while promoting', async () => {
    const { fixture, httpMock } = setup()
    fixture.detectChanges()
    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: false }))
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([])

    await fixture.componentInstance.setSuperAdmin()

    const req = httpMock.expectOne('/api/users/user-2/super-admin')
    req.flush({ error: 'server error' }, { status: 500, statusText: 'Internal Server Error' })
    fixture.detectChanges()

    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
      variant: 'error',
      message: 'Impossible de modifier le statut super-administrateur.',
    })
  })

  it('keeps the super-admin rights when their removal is cancelled', async () => {
    const { fixture, httpMock } = setup({ confirmed: false })
    fixture.detectChanges()
    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: true }))
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([])

    await fixture.componentInstance.setSuperAdmin()

    httpMock.expectNone('/api/users/user-2/super-admin')
  })

  it('grants a new permission for the chosen repository and role', () => {
    const { fixture, httpMock } = setup()
    fixture.detectChanges()
    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: false }))
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([
      {
        id: 'repo-5',
        name: 'new-repo',
        format: 'npm',
        repo_type: 'hosted',
        remote_url: null,
        group_members: [],
      },
    ])
    fixture.detectChanges()

    fixture.componentInstance.grantRepositoryIds.set(['repo-5'])
    fixture.componentInstance.grantRole.set('write')
    fixture.componentInstance.grantPermission()

    const req = httpMock.expectOne('/api/repositories/repo-5/permissions/user-2')
    expect(req.request.method).toBe('PUT')
    expect(req.request.body).toEqual({ role: 'write' })
    req.flush(null)

    // user list is cached and unchanged by a permission edit, so no re-fetch here
    httpMock
      .expectOne('/api/users/user-2/permissions')
      .flush([
        { repository_id: 'repo-5', repository_name: 'new-repo', format: 'npm', role: 'write' },
      ])

    expect(fixture.componentInstance.permissions()).toEqual([
      { repository_id: 'repo-5', repository_name: 'new-repo', format: 'npm', role: 'write' },
    ])
  })

  it('grants the same role to several repositories in one action', () => {
    const { fixture, httpMock } = setup()
    fixture.detectChanges()
    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: false }))
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([
      {
        id: 'repo-5',
        name: 'repo-a',
        format: 'npm',
        repo_type: 'hosted',
        remote_url: null,
        group_members: [],
      },
      {
        id: 'repo-6',
        name: 'repo-b',
        format: 'npm',
        repo_type: 'hosted',
        remote_url: null,
        group_members: [],
      },
    ])
    fixture.detectChanges()

    fixture.componentInstance.grantRepositoryIds.set(['repo-5', 'repo-6'])
    fixture.componentInstance.grantRole.set('read')
    fixture.componentInstance.grantPermission()

    const reqA = httpMock.expectOne('/api/repositories/repo-5/permissions/user-2')
    const reqB = httpMock.expectOne('/api/repositories/repo-6/permissions/user-2')
    expect(reqA.request.body).toEqual({ role: 'read' })
    expect(reqB.request.body).toEqual({ role: 'read' })
    reqA.flush(null)
    reqB.flush(null)

    // user list is cached and unchanged by a permission edit, so no re-fetch here
    httpMock.expectOne('/api/users/user-2/permissions').flush([
      { repository_id: 'repo-5', repository_name: 'repo-a', format: 'npm', role: 'read' },
      { repository_id: 'repo-6', repository_name: 'repo-b', format: 'npm', role: 'read' },
    ])

    expect(fixture.componentInstance.grantRepositoryIds()).toEqual([])
    expect(fixture.componentInstance.permissions().length).toBe(2)
  })

  it('shows an error and still reloads when one grant in a bulk action fails', () => {
    const { fixture, httpMock } = setup()
    fixture.detectChanges()
    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: false }))
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([
      {
        id: 'repo-5',
        name: 'repo-a',
        format: 'npm',
        repo_type: 'hosted',
        remote_url: null,
        group_members: [],
      },
      {
        id: 'repo-6',
        name: 'repo-b',
        format: 'npm',
        repo_type: 'hosted',
        remote_url: null,
        group_members: [],
      },
    ])
    fixture.detectChanges()

    fixture.componentInstance.grantRepositoryIds.set(['repo-5', 'repo-6'])
    fixture.componentInstance.grantRole.set('read')
    fixture.componentInstance.grantPermission()

    httpMock.expectOne('/api/repositories/repo-5/permissions/user-2').flush(null)
    httpMock
      .expectOne('/api/repositories/repo-6/permissions/user-2')
      .flush('error', { status: 500, statusText: 'Server Error' })

    // the successful grant against repo-5 already committed server-side, so reload still runs
    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: false }))
    httpMock
      .expectOne('/api/users/user-2/permissions')
      .flush([{ repository_id: 'repo-5', repository_name: 'repo-a', format: 'npm', role: 'read' }])

    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({ variant: 'error' })
    expect(fixture.componentInstance.permissions().length).toBe(1)
  })

  it('does nothing when no repository is selected for the bulk grant', () => {
    const { fixture, httpMock } = setup()
    fixture.detectChanges()
    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: false }))
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([])
    fixture.detectChanges()

    fixture.componentInstance.grantPermission()

    httpMock.expectNone((r) => r.url.includes('/permissions/user-2'))
  })

  it('sets the shared page title to the username once it loads', () => {
    const { fixture, httpMock } = setup()
    const pageTitle = TestBed.inject(PageTitleService)
    fixture.detectChanges()
    expect(pageTitle.title()).toBe('')

    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: false }))
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([])
    fixture.detectChanges()

    expect(pageTitle.title()).toBe('florian')
  })

  it('shows a load-failure message and hides the management actions when the request errors', () => {
    const { fixture, httpMock } = setup()
    fixture.detectChanges()

    httpMock.expectOne('/api/users/user-2').flush('nope', { status: 403, statusText: 'Forbidden' })
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([])
    fixture.detectChanges()

    expect(fixture.componentInstance.loadError()).toBe(true)
    expect(fixture.nativeElement.querySelector('[role="alert"]')?.textContent).toContain(
      'Impossible de charger cet utilisateur.',
    )
    expect(fixture.nativeElement.textContent).not.toContain('Supprimer')
  })

  describe('as an organization admin (not a super-admin)', () => {
    it('hides the super-admin promotion control entirely', () => {
      const { fixture, httpMock } = setup({ isSuperAdmin: false })
      fixture.detectChanges()
      httpMock
        .expectOne('/api/users/user-2')
        .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: false }))
      httpMock.expectOne('/api/users/user-2/permissions').flush([])
      httpMock.expectOne('/api/repositories').flush([])
      fixture.detectChanges()

      expect(fixture.nativeElement.textContent).not.toContain('super-administrateur')
    })

    it('still shows delete and resend-invitation for a regular (non-super-admin) target', () => {
      const { fixture, httpMock } = setup({ isSuperAdmin: false })
      fixture.detectChanges()
      httpMock.expectOne('/api/users/user-2').flush(
        userFixture({
          id: 'user-2',
          username: 'florian',
          is_super_admin: false,
          invitation_pending: true,
        }),
      )
      httpMock.expectOne('/api/users/user-2/permissions').flush([])
      httpMock.expectOne('/api/repositories').flush([])
      fixture.detectChanges()

      expect(fixture.nativeElement.querySelector('.user-detail__delete')).not.toBeNull()
      expect(fixture.componentInstance.actions()?.canResend).toBe(true)
    })

    it('hides delete and resend-invitation when the target is a super-admin — a global privilege outside their reach', () => {
      const { fixture, httpMock } = setup({ isSuperAdmin: false })
      fixture.detectChanges()
      httpMock.expectOne('/api/users/user-2').flush(
        userFixture({
          id: 'user-2',
          username: 'florian',
          is_super_admin: true,
          invitation_pending: true,
        }),
      )
      httpMock.expectOne('/api/users/user-2/permissions').flush([])
      httpMock.expectOne('/api/repositories').flush([])
      fixture.detectChanges()

      expect(fixture.nativeElement.querySelector('.user-detail__delete')).toBeNull()
      expect(fixture.componentInstance.actions()).toBeNull()
    })
  })

  it('explains on the side what the roles allow and what deleting the account does', () => {
    const { fixture, httpMock } = setup()
    fixture.detectChanges()
    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', is_super_admin: false }))
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([])
    fixture.detectChanges()

    const aside = fixture.nativeElement.querySelector('.user-detail__aside').textContent
    expect(aside).toContain('Les rôles se cumulent')
    expect(aside).toContain('Supprimer un compte le déconnecte aussitôt')
  })

  it('says so when the account does not exist', () => {
    const { fixture, httpMock } = setup()
    fixture.detectChanges()
    httpMock
      .expectOne('/api/users/user-2')
      .flush('not found', { status: 404, statusText: 'Not Found' })
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([])
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain('Utilisateur introuvable')
    expect(fixture.nativeElement.querySelector('a[href="/users"]')).not.toBeNull()
  })

  it("sends one's own account to « Mon compte », as FerrisGit does", () => {
    const { fixture, httpMock } = setup()
    fixture.detectChanges()
    httpMock.expectOne('/api/users/user-2').flush(userFixture({ id: 'user-2', username: 'admin' }))
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([])
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain("C'est votre propre compte")
    expect(fixture.nativeElement.querySelector('a[href="/account"]')).not.toBeNull()
    expect(fixture.nativeElement.querySelector('.user-detail__delete')).toBeNull()
  })

  it('shows the state, second factor and creation date beside the name', () => {
    const { fixture, httpMock } = setup()
    fixture.detectChanges()
    httpMock.expectOne('/api/users/user-2').flush(
      userFixture({
        id: 'user-2',
        username: 'florian',
        email: 'florian@example.com',
        mfa_enabled: true,
      }),
    )
    httpMock
      .expectOne('/api/users/user-2/permissions')
      .flush([{ repository_id: 'repo-1', repository_name: 'my-repo', format: 'npm', role: 'read' }])
    httpMock.expectOne('/api/repositories').flush([])
    fixture.detectChanges()

    const text = fixture.nativeElement.textContent
    expect(text).toContain('Actif')
    expect(text).toContain('Double authentification active')
    expect(text).toContain('florian@example.com')
    expect(text).toContain('créé le 22/08/2026')
    expect(text).toContain('1 dépôt')
  })

  it('asks before removing an access from its row', async () => {
    const { fixture, httpMock, ask } = setup()
    fixture.detectChanges()
    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian' }))
    httpMock
      .expectOne('/api/users/user-2/permissions')
      .flush([{ repository_id: 'repo-1', repository_name: 'my-repo', format: 'npm', role: 'read' }])
    httpMock.expectOne('/api/repositories').flush([])

    await fixture.componentInstance.revokePermission(fixture.componentInstance.permissions()[0])

    expect(ask).toHaveBeenCalledWith(
      expect.objectContaining({ message: "florian n'aura plus accès à my-repo." }),
    )
    const request = httpMock.expectOne('/api/repositories/repo-1/permissions/user-2')
    expect(request.request.method).toBe('DELETE')
  })

  it('offers to grant only the repositories not reached yet', () => {
    const { fixture, httpMock } = setup()
    fixture.detectChanges()
    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian' }))
    httpMock
      .expectOne('/api/users/user-2/permissions')
      .flush([{ repository_id: 'repo-1', repository_name: 'my-repo', format: 'npm', role: 'read' }])
    httpMock.expectOne('/api/repositories').flush([
      { id: 'repo-1', name: 'my-repo' },
      { id: 'repo-2', name: 'other-repo' },
    ])

    expect(fixture.componentInstance.repositoryOptions()).toEqual([
      { value: 'repo-2', label: 'other-repo' },
    ])
  })

  it('resets the password from the actions menu and hands back the link when no mail went out', async () => {
    const { fixture, httpMock, ask } = setup()
    fixture.detectChanges()
    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian' }))
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([])

    expect(fixture.componentInstance.actions()?.canResetPassword).toBe(true)
    await fixture.componentInstance.resetPassword()

    expect(ask).toHaveBeenCalledWith(
      expect.objectContaining({ heading: 'Réinitialiser le mot de passe' }),
    )
    httpMock.expectOne('/api/users/user-2/reset-password').flush({
      email_sent: false,
      email_error: 'email_send_failed',
      reset_url: 'https://app.example.com/reset-password#token=abc',
    })
    fixture.detectChanges()
    expect(fixture.nativeElement.querySelector('app-link-mail-failed code')?.textContent).toBe(
      'https://app.example.com/reset-password#token=abc',
    )
  })

  it('resets the second factors from the actions menu once confirmed', async () => {
    const { fixture, httpMock, ask } = setup()
    fixture.detectChanges()
    httpMock
      .expectOne('/api/users/user-2')
      .flush(userFixture({ id: 'user-2', username: 'florian', mfa_enabled: true }))
    httpMock.expectOne('/api/users/user-2/permissions').flush([])
    httpMock.expectOne('/api/repositories').flush([])

    expect(fixture.componentInstance.actions()?.canResetMfa).toBe(true)
    await fixture.componentInstance.resetMfa()

    expect(ask).toHaveBeenCalledWith(
      expect.objectContaining({ heading: 'Réinitialiser la double authentification' }),
    )
    httpMock.expectOne('/api/users/user-2/mfa').flush(null)
    expect(TestBed.inject(ToastService).toasts().at(-1)?.message).toBe(
      'Double authentification réinitialisée.',
    )
  })

  describe('resending an invitation', () => {
    function loadPending() {
      const context = setup()
      context.fixture.detectChanges()
      context.httpMock.expectOne('/api/users/user-2').flush(
        userFixture({
          id: 'user-2',
          username: 'invite-0a1b2c',
          email: 'dave@example.com',
          invitation_pending: true,
          invitation_expires_at: '2026-10-07T08:00:00Z',
        }),
      )
      context.httpMock.expectOne('/api/users/user-2/permissions').flush([])
      context.httpMock.expectOne('/api/repositories').flush([])
      context.fixture.detectChanges()
      return context
    }

    function answerReload(httpMock: HttpTestingController) {
      httpMock.match('/api/users/user-2').forEach((request) =>
        request.flush(
          userFixture({
            id: 'user-2',
            username: 'x',
            invitation_pending: true,
            email: 'dave@example.com',
          }),
        ),
      )
      httpMock.match('/api/users/user-2/permissions').forEach((request) => request.flush([]))
    }

    it('says to whom it went', () => {
      const { fixture, httpMock } = loadPending()
      const toast = vi.spyOn(TestBed.inject(ToastService), 'success')

      fixture.componentInstance.resendInvitation()
      httpMock.expectOne('/api/users/user-2/resend-invitation').flush({ email_sent: true })
      answerReload(httpMock)

      expect(toast).toHaveBeenCalledWith('Invitation renvoyée à dave@example.com.')
    })

    it('shows the new activation link when its mail could not go out', () => {
      const { fixture, httpMock } = loadPending()

      fixture.componentInstance.resendInvitation()
      httpMock.expectOne('/api/users/user-2/resend-invitation').flush({
        email_sent: false,
        email_error: 'email_not_configured',
        activation_url: 'https://app.example.com/activate#token=new',
      })
      answerReload(httpMock)
      fixture.detectChanges()

      const alert = fixture.nativeElement.querySelector('app-link-mail-failed')
      expect(alert?.textContent).toContain("Aucun serveur mail n'est configuré")
      expect(alert?.querySelector('code')?.textContent).toBe(
        'https://app.example.com/activate#token=new',
      )
    })
  })

  describe('navigating from one user to another', () => {
    const PERMISSION_A = {
      repository_id: 'repo-a',
      repository_name: 'a-repo',
      format: 'npm',
      role: 'write',
    }

    function setupNavigable() {
      const paramMap$ = new BehaviorSubject(convertToParamMap({ id: 'user-a' }))
      TestBed.configureTestingModule({
        providers: [
          provideHttpClient(),
          provideHttpClientTesting(),
          provideRouter([]),
          ...userProviders,
          ...repositoryProviders,
          {
            provide: ActivatedRoute,
            useValue: { snapshot: { paramMap: paramMap$.value }, paramMap: paramMap$ },
          },
          { provide: MeService, useValue: { isSuperAdmin: () => true, username: () => 'admin' } },
          { provide: ConfirmService, useValue: { ask: vi.fn().mockResolvedValue(true) } },
        ],
      })
      const fixture = TestBed.createComponent(UserDetail)
      const httpMock = TestBed.inject(HttpTestingController)
      fixture.detectChanges()
      httpMock
        .expectOne('/api/users/user-a')
        .flush({ id: 'user-a', username: 'alice', is_super_admin: false })
      httpMock.expectOne('/api/users/user-a/permissions').flush([PERMISSION_A])
      httpMock.expectOne('/api/repositories').flush([])
      fixture.detectChanges()
      const goTo = (id: string) => {
        paramMap$.next(convertToParamMap({ id }))
        fixture.detectChanges()
      }
      return { fixture, httpMock, goTo }
    }

    it("drops user A's data while user B loads", () => {
      const { fixture, httpMock, goTo } = setupNavigable()
      fixture.componentInstance.openRoleEditor(PERMISSION_A as never)
      fixture.componentInstance.grantRepositoryIds.set(['repo-a'])

      goTo('user-b')

      const page = fixture.componentInstance
      expect(page.user()).toBeNull()
      expect(page.permissions()).toEqual([])
      expect(page.editingPermission()).toBeNull()
      expect(page.grantRepositoryIds()).toEqual([])
      expect(fixture.nativeElement.textContent).not.toContain('alice')
      httpMock.match(() => true)
    })

    it("cannot send B's id with A's permission entry", () => {
      const { fixture, httpMock, goTo } = setupNavigable()
      fixture.componentInstance.openRoleEditor(PERMISSION_A as never)

      goTo('user-b')
      fixture.componentInstance.changeRole('read')
      fixture.componentInstance.revokeFromEditor()

      httpMock.expectNone((req) => req.url.includes('/permissions/user-b'))
      httpMock.match(() => true)
    })

    it("does not let A's late grant answer touch B's page", () => {
      const { fixture, httpMock, goTo } = setupNavigable()
      fixture.componentInstance.openRoleEditor(PERMISSION_A as never)
      fixture.componentInstance.changeRole('read')
      const grantA = httpMock.expectOne('/api/repositories/repo-a/permissions/user-a')

      goTo('user-b')
      httpMock.expectOne('/api/users/user-b').flush({ id: 'user-b', username: 'bob' })
      httpMock.expectOne('/api/users/user-b/permissions').flush([])
      grantA.flush(null)

      httpMock.expectNone('/api/users/user-a/permissions')
      httpMock.expectNone('/api/users/user-b/permissions')
      expect(fixture.componentInstance.permissions()).toEqual([])
      expect(fixture.componentInstance.savingRole()).toBe(false)
    })

    it('shows the load error when the permissions request fails', () => {
      const { fixture, httpMock, goTo } = setupNavigable()

      goTo('user-b')
      httpMock.expectOne('/api/users/user-b').flush({ id: 'user-b', username: 'bob' })
      httpMock
        .expectOne('/api/users/user-b/permissions')
        .flush({}, { status: 500, statusText: 'Error' })
      fixture.detectChanges()

      expect(fixture.componentInstance.loadError()).toBe(true)
      expect(fixture.nativeElement.querySelector('[role="alert"]')).toBeTruthy()
    })
  })
})
