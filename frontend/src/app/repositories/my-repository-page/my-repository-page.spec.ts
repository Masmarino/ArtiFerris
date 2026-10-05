import { vi } from 'vitest'
import { TestBed } from '@angular/core/testing'
import { HttpErrorResponse, provideHttpClient } from '@angular/common/http'
import { provideHttpClientTesting } from '@angular/common/http/testing'
import { provideRouter } from '@angular/router'
import { By } from '@angular/platform-browser'
import { Button } from '@masmarino/gabarit/button'
import { of, throwError } from 'rxjs'
import { MyRepositoryPage } from './my-repository-page'
import { PersonalRepositoryService } from '../application/personal-repository.service'
import { RepositoriesList } from '../repositories-list/repositories-list'
import { ConfirmModal } from '../../shared/confirm-modal/confirm-modal'
import { repositoryProviders } from '../infrastructure/repository.providers'
import { organizationsProviders } from '../../admin/infrastructure/organizations.providers'
import { MeService } from '../../shell/application/me.service'
import { ToastService } from '../../shared/toast.service'

function render(overrides: Partial<PersonalRepositoryService> = {}) {
  const service: Partial<PersonalRepositoryService> = {
    hasReservedNamespace: () => of(false),
    reserve: () => of(undefined),
    listMyProjects: () => of([]),
    ...overrides,
  }
  TestBed.configureTestingModule({
    providers: [
      provideHttpClient(),
      provideHttpClientTesting(),
      provideRouter([]),
      ...repositoryProviders,
      ...organizationsProviders,
      { provide: PersonalRepositoryService, useValue: service },
      { provide: MeService, useValue: { isSuperAdmin: () => false } },
    ],
  })
  const fixture = TestBed.createComponent(MyRepositoryPage)
  fixture.detectChanges()
  return { fixture }
}

describe('MyRepositoryPage', () => {
  it('calls hasReservedNamespace() on init', () => {
    const hasReservedNamespace = vi.fn().mockReturnValue(of(false))
    render({ hasReservedNamespace })

    expect(hasReservedNamespace).toHaveBeenCalled()
  })

  describe('when the namespace is not yet reserved', () => {
    it('shows the create button and hides RepositoriesList/ConfirmModal', () => {
      const { fixture } = render({ hasReservedNamespace: () => of(false) })

      const button = fixture.debugElement.query(By.directive(Button)).componentInstance as Button
      expect(button.text()).toBe('Créer son dépôt utilisateur')
      expect(fixture.debugElement.query(By.directive(RepositoriesList))).toBeFalsy()
      expect(fixture.debugElement.query(By.directive(ConfirmModal))).toBeFalsy()
    })

    it('opens the ConfirmModal when the create button is clicked', () => {
      const { fixture } = render({ hasReservedNamespace: () => of(false) })

      fixture.nativeElement.querySelector('gbt-button').dispatchEvent(new CustomEvent('clicked'))
      fixture.detectChanges()

      expect(fixture.debugElement.query(By.directive(ConfirmModal))).toBeTruthy()
    })

    it('calls reserve() and then shows RepositoriesList in personal mode once confirmed', () => {
      const reserve = vi.fn().mockReturnValue(of(undefined))
      const { fixture } = render({ hasReservedNamespace: () => of(false), reserve })

      fixture.nativeElement.querySelector('gbt-button').dispatchEvent(new CustomEvent('clicked'))
      fixture.detectChanges()
      const modal = fixture.debugElement.query(By.directive(ConfirmModal))
      modal.componentInstance.confirmed.emit()
      fixture.detectChanges()

      expect(reserve).toHaveBeenCalled()
      expect(fixture.debugElement.query(By.directive(ConfirmModal))).toBeFalsy()
      const list = fixture.debugElement.query(By.directive(RepositoriesList))
      expect(list).toBeTruthy()
      expect(list.componentInstance.mode()).toBe('personal')
    })

    it('closes the modal without calling reserve() when cancelled', () => {
      const reserve = vi.fn().mockReturnValue(of(undefined))
      const { fixture } = render({ hasReservedNamespace: () => of(false), reserve })

      fixture.nativeElement.querySelector('gbt-button').dispatchEvent(new CustomEvent('clicked'))
      fixture.detectChanges()
      const modal = fixture.debugElement.query(By.directive(ConfirmModal))
      modal.componentInstance.cancelled.emit()
      fixture.detectChanges()

      expect(reserve).not.toHaveBeenCalled()
      expect(fixture.debugElement.query(By.directive(ConfirmModal))).toBeFalsy()
      expect(fixture.debugElement.query(By.directive(RepositoriesList))).toBeFalsy()
    })

    it('shows an error toast and keeps the modal open, re-enabling confirm, when reserve() fails', () => {
      const reserve = vi
        .fn()
        .mockReturnValue(
          throwError(() => new HttpErrorResponse({ status: 400, error: { error: 'échec' } })),
        )
      const { fixture } = render({ hasReservedNamespace: () => of(false), reserve })

      fixture.nativeElement.querySelector('gbt-button').dispatchEvent(new CustomEvent('clicked'))
      fixture.detectChanges()
      const modal = fixture.debugElement.query(By.directive(ConfirmModal))
      modal.componentInstance.confirmed.emit()
      fixture.detectChanges()

      expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
        variant: 'error',
        message: 'échec',
      })
      const stillOpenModal = fixture.debugElement.query(By.directive(ConfirmModal))
      expect(stillOpenModal).toBeTruthy()
      expect(stillOpenModal.componentInstance.confirming()).toBe(false)
      expect(fixture.debugElement.query(By.directive(RepositoriesList))).toBeFalsy()
    })

    it('treats a 409 from reserve() as success instead of stranding the user', () => {
      const reserve = vi
        .fn()
        .mockReturnValue(throwError(() => new HttpErrorResponse({ status: 409 })))
      const { fixture } = render({ hasReservedNamespace: () => of(false), reserve })

      fixture.nativeElement.querySelector('gbt-button').dispatchEvent(new CustomEvent('clicked'))
      fixture.detectChanges()
      const modal = fixture.debugElement.query(By.directive(ConfirmModal))
      modal.componentInstance.confirmed.emit()
      fixture.detectChanges()

      expect(fixture.debugElement.query(By.directive(ConfirmModal))).toBeFalsy()
      const list = fixture.debugElement.query(By.directive(RepositoriesList))
      expect(list).toBeTruthy()
      expect(list.componentInstance.mode()).toBe('personal')
    })
  })

  describe('when the namespace is already reserved', () => {
    it('renders RepositoriesList in personal mode immediately, without ever showing the create button', () => {
      const { fixture } = render({ hasReservedNamespace: () => of(true) })

      const list = fixture.debugElement.query(By.directive(RepositoriesList))
      expect(list).toBeTruthy()
      expect(list.componentInstance.mode()).toBe('personal')
      expect(fixture.nativeElement.textContent).not.toContain('Créer son dépôt utilisateur')
      expect(fixture.debugElement.query(By.directive(ConfirmModal))).toBeFalsy()
    })
  })
})
