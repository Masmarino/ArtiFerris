import {
  catalogEntry,
  catalogSuggestion,
  dockerEntry,
  dockerSuggestion,
} from '../testing/catalog.fixtures'
import { ownerLink, packageLink, repositoryLink } from './catalog-links'

describe('catalog links', () => {
  it('links a personal owner to the @username profile', () => {
    expect(ownerLink({ kind: 'personal', slug: 'admin' })).toEqual(['/@admin'])
  })

  it('links an organization owner to /o/:slug', () => {
    expect(ownerLink({ kind: 'organization', slug: 'acme' })).toEqual(['/o', 'acme'])
  })

  it('accepts a full catalog owner too', () => {
    expect(ownerLink(catalogEntry().owner)).toEqual(['/@admin'])
  })

  it('links a personal owner to the @username repository page', () => {
    expect(repositoryLink(catalogEntry())).toEqual(['/@admin', 'test-npm'])
  })

  it('links an organization owner under /o/:slug', () => {
    expect(repositoryLink(dockerEntry())).toEqual(['/o', 'acme', 'images'])
  })

  it('appends packages/<kind>/<name> to the repository link', () => {
    expect(packageLink(catalogEntry())).toEqual([
      '/@admin',
      'test-npm',
      'packages',
      'npm',
      'hangar-demo',
    ])
    expect(packageLink(dockerEntry())).toEqual([
      '/o',
      'acme',
      'images',
      'packages',
      'docker',
      'team/api',
    ])
  })

  it('links a suggestion to its package page like a search result', () => {
    expect(packageLink(catalogSuggestion())).toEqual(packageLink(catalogEntry()))
    expect(packageLink(dockerSuggestion())).toEqual(packageLink(dockerEntry()))
  })

  it('links a personal suggestion under the @username', () => {
    expect(packageLink(catalogSuggestion({ name: 'left-pad' }))).toEqual([
      '/@admin',
      'test-npm',
      'packages',
      'npm',
      'left-pad',
    ])
  })

  it('links an organization suggestion under /o/:slug', () => {
    expect(packageLink(dockerSuggestion())).toEqual([
      '/o',
      'acme',
      'images',
      'packages',
      'docker',
      'team/api',
    ])
  })

  it('keeps scoped and slashed names as a single raw segment, for the router to encode', () => {
    const link = packageLink(catalogEntry({ name: '@scope/pkg' }))

    expect(link[link.length - 1]).toBe('@scope/pkg')
  })
})
