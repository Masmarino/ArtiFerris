import { TestBed } from '@angular/core/testing'
import { By } from '@angular/platform-browser'
import { provideHttpClient } from '@angular/common/http'
import { provideHttpClientTesting } from '@angular/common/http/testing'
import { Select } from '@masmarino/gabarit/select'
import { of, throwError } from 'rxjs'
import { LanguageSettings } from './language-settings'
import { MeService } from '../../shell/application/me.service'
import { ME_PORT, MePort } from '../../shell/application/me.port'
import { authProviders } from '../../auth/infrastructure/auth.providers'
import { LanguageService } from '../../shared/i18n/language.service'
import { setActiveLanguage } from '../../shared/i18n/translator'
import { ToastService } from '../../shared/toast.service'

describe('LanguageSettings', () => {
  function render(port: Partial<MePort> = { setLanguage: () => of(undefined) }) {
    TestBed.configureTestingModule({
      imports: [LanguageSettings],
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...authProviders,
        { provide: ME_PORT, useValue: port },
      ],
    })
    const language = TestBed.inject(LanguageService)
    const use = vi.spyOn(language, 'use').mockImplementation(async (next) => {
      setActiveLanguage(next)
    })
    const fixture = TestBed.createComponent(LanguageSettings)
    fixture.detectChanges()
    const select = () =>
      fixture.debugElement.query(By.directive(Select)).componentInstance as Select<string>
    return {
      fixture,
      use,
      component: fixture.componentInstance,
      select,
      toast: TestBed.inject(ToastService),
    }
  }

  afterEach(() => setActiveLanguage('fr'))

  it('offers every language, each named in itself', () => {
    const { component } = render()

    expect(component.options).toEqual([
      { value: 'en', label: 'English' },
      { value: 'fr', label: 'Français' },
      { value: 'es', label: 'Español' },
      { value: 'it', label: 'Italiano' },
      { value: 'de', label: 'Deutsch' },
    ])
  })

  it('shows the language currently displayed as the selection', () => {
    setActiveLanguage('de')
    const { component } = render()

    expect(component.current()).toBe('de')
  })

  it('applies the chosen language at once and saves it on the account', async () => {
    const setLanguage = vi.fn().mockReturnValue(of(undefined))
    const { component, use, toast } = render({ setLanguage })
    const success = vi.spyOn(toast, 'success')

    await component.choose('it')

    expect(use).toHaveBeenCalledWith('it')
    expect(setLanguage).toHaveBeenCalledWith('it')
    expect(TestBed.inject(MeService).language()).toBe('it')
    expect(success).toHaveBeenCalledWith('Langue enregistrée.')
  })

  it('goes back to the previous language, and says so, when the save fails', async () => {
    const { component, use, toast } = render({
      setLanguage: () => throwError(() => new Error('500')),
    })
    const error = vi.spyOn(toast, 'error')

    await component.choose('de')

    expect(use.mock.calls.map(([language]) => language)).toEqual(['de', 'fr'])
    expect(component.current()).toBe('fr')
    expect(error).toHaveBeenCalledWith("Échec de l'enregistrement de la langue. Réessayez.")
    expect(component.saving()).toBe(false)
  })

  it.each(['fr', 'xx', ''])('does nothing for %j', async (value) => {
    const setLanguage = vi.fn().mockReturnValue(of(undefined))
    const { component, use } = render({ setLanguage })

    await component.choose(value)

    expect(use).not.toHaveBeenCalled()
    expect(setLanguage).not.toHaveBeenCalled()
  })

  it('is driven by its select', async () => {
    const setLanguage = vi.fn().mockReturnValue(of(undefined))
    const { fixture } = render({ setLanguage })
    const select = fixture.debugElement.query(By.directive(Select))

    select.triggerEventHandler('ngModelChange', 'es')
    await fixture.whenStable()

    expect(setLanguage).toHaveBeenCalledWith('es')
  })
})
