import { TestBed } from '@angular/core/testing'
import { of } from 'rxjs'
import { PersonalRepositoryService } from './personal-repository.service'
import { PERSONAL_REPOSITORY_PORT, PersonalRepositoryPort } from './personal-repository.port'

describe('PersonalRepositoryService', () => {
  function setup(port: Partial<PersonalRepositoryPort>) {
    TestBed.configureTestingModule({
      providers: [{ provide: PERSONAL_REPOSITORY_PORT, useValue: port }],
    })
    return TestBed.inject(PersonalRepositoryService)
  }

  it('delegates hasReservedNamespace() to the port', () => {
    const checkReserved = vi.fn().mockReturnValue(of(true))
    let result: boolean | undefined
    setup({ checkReserved })
      .hasReservedNamespace()
      .subscribe((value) => (result = value))

    expect(checkReserved).toHaveBeenCalled()
    expect(result).toBe(true)
  })

  it('resolves false when the port reports no reserved namespace', () => {
    const checkReserved = vi.fn().mockReturnValue(of(false))
    let result: boolean | undefined
    setup({ checkReserved })
      .hasReservedNamespace()
      .subscribe((value) => (result = value))

    expect(result).toBe(false)
  })

  it('delegates reserve() to the port', () => {
    const reserve = vi.fn().mockReturnValue(of(undefined))
    setup({ reserve }).reserve()

    expect(reserve).toHaveBeenCalled()
  })

  it('delegates listMyProjects() to the port', () => {
    const listMyProjects = vi.fn().mockReturnValue(of([]))
    setup({ listMyProjects }).listMyProjects()

    expect(listMyProjects).toHaveBeenCalled()
  })

  it('delegates createProject() to the port', () => {
    const createProject = vi.fn().mockReturnValue(of({}))
    setup({ createProject }).createProject('my-project', 'npm', 'hosted')

    expect(createProject).toHaveBeenCalledWith('my-project', 'npm', 'hosted')
  })
})
