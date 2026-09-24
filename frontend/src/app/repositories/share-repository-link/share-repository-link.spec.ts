import { DOCUMENT } from '@angular/common'
import { TestBed } from '@angular/core/testing'
import { provideRouter } from '@angular/router'
import { ShareRepositoryLink } from './share-repository-link'
import { RepositorySummary } from '../domain/repository.entity'

const FAKE_DOCUMENT = {
  location: { origin: 'https://artiferris.example' },
  createElement: (tag: string) => document.createElement(tag),
  createElementNS: (ns: string | null, tag: string) => document.createElementNS(ns, tag),
  createTextNode: (text: string) => document.createTextNode(text),
  createComment: (text: string) => document.createComment(text),
  querySelector: (selector: string) => document.querySelector(selector),
  body: document.body,
}

function repo(overrides: Partial<RepositorySummary>): RepositorySummary {
  return {
    id: 'repo-1',
    name: 'libs',
    format: 'npm',
    repo_type: 'hosted',
    remote_url: null,
    remote_credentials_set: false,
    group_members: [],
    quota_bytes: null,
    retention_keep_last_n: null,
    is_public: true,
    my_role: 'admin',
    organization_id: 'org-1',
    owner_name: 'alice',
    owner_is_personal: true,
    public_path: '/@alice/libs',
    ...overrides,
  }
}

function render(repository: RepositorySummary, canManageVisibility = false) {
  TestBed.configureTestingModule({
    providers: [provideRouter([]), { provide: DOCUMENT, useValue: FAKE_DOCUMENT }],
  })
  const fixture = TestBed.createComponent(ShareRepositoryLink)
  fixture.componentRef.setInput('repository', repository)
  fixture.componentRef.setInput('canManageVisibility', canManageVisibility)
  fixture.detectChanges()
  return fixture
}

describe('ShareRepositoryLink', () => {
  it('shows the full public URL and a link to the public page for a public repository', () => {
    const fixture = render(repo({}))

    expect(fixture.nativeElement.textContent).toContain('https://artiferris.example/@alice/libs')
    const link: HTMLAnchorElement = fixture.nativeElement.querySelector('a[href="/@alice/libs"]')
    expect(link).toBeTruthy()
    expect(link.textContent).toContain('Voir la page publique')
  })

  it('builds an organization repository link from its public_path as given', () => {
    const fixture = render(
      repo({ owner_is_personal: false, owner_name: 'Acme Corp', public_path: '/o/acme/libs' }),
    )

    expect(fixture.nativeElement.textContent).toContain('https://artiferris.example/o/acme/libs')
  })

  it('shows guidance instead of a link when the repository is private', () => {
    const fixture = render(repo({ is_public: false, public_path: null }))

    expect(fixture.nativeElement.textContent).toContain('Ce dépôt est privé')
    expect(fixture.nativeElement.textContent).not.toContain('https://')
    expect(fixture.nativeElement.querySelector('a')).toBeNull()
  })

  it('does not mention the Paramètres tab to a viewer who cannot reach it', () => {
    const fixture = render(repo({ is_public: false, public_path: null }))

    expect(fixture.nativeElement.textContent).not.toContain('Paramètres')
  })

  it('points a viewer who can manage visibility at the Paramètres tab', () => {
    const fixture = render(repo({ is_public: false, public_path: null }), true)

    expect(fixture.nativeElement.textContent).toContain('Paramètres')
  })

  it('treats a missing public_path (an older fixture) the same as private', () => {
    const fixture = render(repo({ public_path: undefined }))

    expect(fixture.nativeElement.textContent).toContain('Ce dépôt est privé')
  })
})
