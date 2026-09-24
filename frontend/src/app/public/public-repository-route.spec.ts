import { TestBed } from '@angular/core/testing'
import { convertToParamMap } from '@angular/router'
import { of } from 'rxjs'
import { RepositoriesService } from '../repositories/application/repositories.service'
import { publicRepositoryBasePath, resolvePublicRepository } from './public-repository-route'

const personal = convertToParamMap({ username: '@alice', repoName: 'my-lib' })
const organization = convertToParamMap({ slug: 'acme', repoName: 'my-lib' })

describe('public repository route helpers', () => {
  function service() {
    const getByOwner = vi.fn().mockReturnValue(of({ id: 'personal' }))
    const getByOrg = vi.fn().mockReturnValue(of({ id: 'org' }))
    TestBed.configureTestingModule({
      providers: [{ provide: RepositoriesService, useValue: { getByOwner, getByOrg } }],
    })
    return { repositories: TestBed.inject(RepositoriesService), getByOwner, getByOrg }
  }

  it('resolves a personal route through by-owner, without the @', () => {
    const { repositories, getByOwner, getByOrg } = service()

    resolvePublicRepository(repositories, personal).subscribe()

    expect(getByOwner).toHaveBeenCalledWith('alice', 'my-lib')
    expect(getByOrg).not.toHaveBeenCalled()
  })

  it('resolves an organization route through by-org', () => {
    const { repositories, getByOwner, getByOrg } = service()

    resolvePublicRepository(repositories, organization).subscribe()

    expect(getByOrg).toHaveBeenCalledWith('acme', 'my-lib')
    expect(getByOwner).not.toHaveBeenCalled()
  })

  it('keeps the raw @ segment in the personal base path', () => {
    expect(publicRepositoryBasePath(personal)).toEqual(['/@alice', 'my-lib'])
  })

  it('builds /o/:slug/:repoName for an organization', () => {
    expect(publicRepositoryBasePath(organization)).toEqual(['/o', 'acme', 'my-lib'])
  })
})
