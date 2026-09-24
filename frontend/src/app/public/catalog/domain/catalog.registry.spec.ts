import { CATALOGS, catalogNameFromUrl } from './catalog.registry'

describe('catalog registry', () => {
  it('mirrors the backend: one catalog per format, named artiferris-<format>', () => {
    expect(CATALOGS.map((c) => [c.format, c.name])).toEqual([
      ['npm', 'artiferris-npm'],
      ['docker', 'artiferris-docker'],
    ])
  })

  it('recognizes a catalog page URL, with or without a query string or fragment', () => {
    expect(catalogNameFromUrl('/artiferris-npm')).toBe('artiferris-npm')
    expect(catalogNameFromUrl('/artiferris-docker?q=web&page=2')).toBe('artiferris-docker')
    expect(catalogNameFromUrl('/artiferris-npm#top')).toBe('artiferris-npm')
  })

  it('returns null for anything else', () => {
    expect(catalogNameFromUrl('/explorer?q=x')).toBeNull()
    expect(catalogNameFromUrl('/artiferris-npm/extra')).toBeNull()
    expect(catalogNameFromUrl('/artiferris-helm')).toBeNull()
    expect(catalogNameFromUrl('/')).toBeNull()
  })
})
