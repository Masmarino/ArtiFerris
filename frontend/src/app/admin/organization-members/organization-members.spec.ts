import { ComponentFixture, TestBed } from '@angular/core/testing'
import { By } from '@angular/platform-browser'
import { Subject, of, throwError } from 'rxjs'
import { OrganizationMember } from '../domain/organization-member.entity'
import { Tooltip } from '@masmarino/gabarit'
import { OrganizationMembers } from './organization-members'
import { OrganizationMembersService } from '../application/organization-members.service'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'

function clickPromoteDemoteButton(fixture: ComponentFixture<OrganizationMembers>): void {
  const button = fixture.debugElement.query(By.css('.organization-members__actions button'))
  button.nativeElement.click()
}

describe('OrganizationMembers', () => {
  let fixture: ComponentFixture<OrganizationMembers>
  let component: OrganizationMembers
  let serviceSpy: {
    list: ReturnType<typeof vi.fn>
    invite: ReturnType<typeof vi.fn>
    setOrganizationAdmin: ReturnType<typeof vi.fn>
  }

  let askSpy: ReturnType<typeof vi.fn>

  function setup() {
    askSpy = vi.fn().mockResolvedValue(true)
    serviceSpy = { list: vi.fn(), invite: vi.fn(), setOrganizationAdmin: vi.fn() }
    serviceSpy.list.mockReturnValue(
      of([
        {
          id: 'user-1',
          username: 'florian',
          email: 'florian@example.com',
          is_organization_admin: false,
          invitation_pending: false,
        },
      ]),
    )
    TestBed.configureTestingModule({
      imports: [OrganizationMembers],
      providers: [
        { provide: OrganizationMembersService, useValue: serviceSpy },
        { provide: ConfirmService, useValue: { ask: askSpy } },
      ],
    })
    fixture = TestBed.createComponent(OrganizationMembers)
    component = fixture.componentInstance
    fixture.componentRef.setInput('organizationId', 'org-1')
    fixture.detectChanges()
  }

  it('loads members for the given organization on init', () => {
    setup()

    expect(serviceSpy.list).toHaveBeenCalledWith('org-1')
    expect(component.members()).toEqual([
      {
        id: 'user-1',
        username: 'florian',
        email: 'florian@example.com',
        is_organization_admin: false,
        invitation_pending: false,
      },
    ])
  })

  it('invites a new member and reloads the list', () => {
    setup()
    serviceSpy.invite.mockReturnValue(
      of({
        id: 'user-2',
        username: 'newmember',
        email: 'newmember@example.com',
        is_organization_admin: false,
        invitation_pending: true,
      }),
    )
    component.startAdding()
    component.newUsername.set('newmember')
    component.newEmail.set('newmember@example.com')

    component.invite()

    expect(serviceSpy.invite).toHaveBeenCalledWith(
      'org-1',
      'newmember',
      'newmember@example.com',
      false,
    )
    expect(serviceSpy.list).toHaveBeenCalledTimes(2)
    expect(component.addingMember()).toBe(false)
    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
      variant: 'success',
      message: 'newmember a été invité·e.',
    })
  })

  it('shows an error toast when the invite fails', () => {
    setup()
    serviceSpy.invite.mockReturnValue(throwError(() => new Error('conflict')))
    component.startAdding()
    component.newUsername.set('newmember')
    component.newEmail.set('newmember@example.com')

    component.invite()

    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
      variant: 'error',
      message: "Échec de l'invitation.",
    })
  })

  it('toggles organization-admin status after confirmation and reloads when the promote/demote button is clicked', async () => {
    setup()
    serviceSpy.setOrganizationAdmin.mockReturnValue(of(undefined))

    clickPromoteDemoteButton(fixture)
    await fixture.whenStable()

    expect(askSpy).toHaveBeenCalledWith(expect.objectContaining({ confirmLabel: 'Promouvoir' }))
    expect(serviceSpy.setOrganizationAdmin).toHaveBeenCalledWith('org-1', 'user-1', true)
    expect(serviceSpy.list).toHaveBeenCalledTimes(2)
    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
      variant: 'success',
      message: "florian est désormais administrateur·rice de l'organisation.",
    })
  })

  it('does nothing when the toggle confirmation is dismissed', async () => {
    setup()
    askSpy.mockResolvedValue(false)

    clickPromoteDemoteButton(fixture)
    await fixture.whenStable()

    expect(askSpy).toHaveBeenCalled()
    expect(serviceSpy.setOrganizationAdmin).not.toHaveBeenCalled()
  })

  it('labels the button "Promouvoir" for a non-admin member and "Rétrograder" for an admin member', () => {
    setup()
    fixture.detectChanges()

    const button = fixture.debugElement.query(By.css('.organization-members__actions button'))
    expect(button.nativeElement.textContent.trim()).toBe('Promouvoir')
  })

  it('does not react to clicks anywhere else on the member row', () => {
    setup()

    const row = fixture.debugElement.query(By.css('.organization-members__table tbody tr'))
    row.nativeElement.click()

    expect(askSpy).not.toHaveBeenCalled()
    expect(serviceSpy.setOrganizationAdmin).not.toHaveBeenCalled()
  })

  it('explains via a tooltip what promoting a non-admin member does', () => {
    setup()

    const tooltip = fixture.debugElement.query(By.directive(Tooltip))

    expect((tooltip.componentInstance as Tooltip).text()).toBe(
      "Donne les droits d'administration complets sur cette organisation.",
    )
  })

  it('explains via a tooltip what demoting an admin member does', () => {
    serviceSpy = { list: vi.fn(), invite: vi.fn(), setOrganizationAdmin: vi.fn() }
    serviceSpy.list.mockReturnValue(
      of([
        {
          id: 'user-1',
          username: 'florian',
          email: 'florian@example.com',
          is_organization_admin: true,
          invitation_pending: false,
        },
      ]),
    )
    TestBed.configureTestingModule({
      imports: [OrganizationMembers],
      providers: [
        { provide: OrganizationMembersService, useValue: serviceSpy },
        { provide: ConfirmService, useValue: { ask: askSpy } },
      ],
    })
    fixture = TestBed.createComponent(OrganizationMembers)
    fixture.componentRef.setInput('organizationId', 'org-1')
    fixture.detectChanges()

    const tooltip = fixture.debugElement.query(By.directive(Tooltip))

    expect((tooltip.componentInstance as Tooltip).text()).toBe(
      "Retire les droits d'administration de cette organisation.",
    )
  })

  it('keeps the current organization when a slower response for the previous one lands last', () => {
    setup()
    const late = new Subject<OrganizationMember[]>()
    const member = (username: string): OrganizationMember => ({
      id: username,
      username,
      email: `${username}@example.com`,
      is_organization_admin: false,
      invitation_pending: false,
    })
    serviceSpy.list.mockReturnValueOnce(late).mockReturnValueOnce(of([member('b-user')]))

    fixture.componentRef.setInput('organizationId', 'org-a')
    fixture.detectChanges()
    fixture.componentRef.setInput('organizationId', 'org-b')
    fixture.detectChanges()
    late.next([member('a-user')])
    fixture.detectChanges()

    expect(component.members().map((m) => m.username)).toEqual(['b-user'])
  })

  it('stops the spinner and reports the failure when loading members fails', () => {
    setup()
    serviceSpy.list.mockReturnValue(throwError(() => new Error('boom')))

    fixture.componentRef.setInput('organizationId', 'org-c')
    fixture.detectChanges()

    expect(component.loading()).toBe(false)
    expect(component.errorMessage()).toBe('Échec du chargement des membres.')
  })

  it('drops a half-typed invite when switching to another organization', () => {
    setup()
    component.startAdding()
    component.newUsername.set('half-typed')
    component.newEmail.set('half@example.com')
    component.newIsOrganizationAdmin.set(true)

    fixture.componentRef.setInput('organizationId', 'org-b')
    fixture.detectChanges()

    expect(component.addingMember()).toBe(false)
    expect(component.newUsername()).toBe('')
    expect(component.newEmail()).toBe('')
    expect(component.newIsOrganizationAdmin()).toBe(false)
    component.invite()
    expect(serviceSpy.invite).not.toHaveBeenCalled()
  })

  it('does not reopen or reload the new organization when an invite for the previous one finishes late', () => {
    setup()
    const pending = new Subject<void>()
    serviceSpy.invite.mockReturnValue(pending)
    component.startAdding()
    component.newUsername.set('alice')
    component.newEmail.set('alice@example.com')
    component.invite()
    fixture.componentRef.setInput('organizationId', 'org-b')
    fixture.detectChanges()
    serviceSpy.list.mockClear()

    pending.next()
    pending.complete()

    expect(serviceSpy.list).not.toHaveBeenCalled()
    expect(TestBed.inject(ToastService).toasts().at(-1)?.message).toContain('alice')
  })
})
