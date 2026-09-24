import { ownerSummary } from '../testing/catalog.fixtures'
import { ownerCountsLabel } from './owner-counts'

describe('ownerCountsLabel', () => {
  it('lists repositories, packages and images', () => {
    expect(ownerCountsLabel(ownerSummary())).toBe('2 dépôts · 3 paquets · 4 images')
  })

  it('uses the singular for exactly one', () => {
    expect(
      ownerCountsLabel(ownerSummary({ repository_count: 1, package_count: 1, image_count: 1 })),
    ).toBe('1 dépôt · 1 paquet · 1 image')
  })

  it('leaves out the parts that are zero', () => {
    expect(ownerCountsLabel(ownerSummary({ package_count: 0 }))).toBe('2 dépôts · 4 images')
    expect(ownerCountsLabel(ownerSummary({ package_count: 0, image_count: 0 }))).toBe('2 dépôts')
    expect(ownerCountsLabel(ownerSummary({ repository_count: 1, image_count: 0 }))).toBe(
      '1 dépôt · 3 paquets',
    )
  })

  it('is empty when everything is zero', () => {
    expect(
      ownerCountsLabel(ownerSummary({ repository_count: 0, package_count: 0, image_count: 0 })),
    ).toBe('')
  })
})
