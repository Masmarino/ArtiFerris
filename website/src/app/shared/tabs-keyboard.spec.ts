import { tabIndexForKey } from './tabs-keyboard'

describe('tabIndexForKey', () => {
  it('moves with the arrows and wraps at both ends', () => {
    expect(tabIndexForKey('ArrowRight', 0, 6)).toBe(1)
    expect(tabIndexForKey('ArrowRight', 5, 6)).toBe(0)
    expect(tabIndexForKey('ArrowLeft', 0, 6)).toBe(5)
    expect(tabIndexForKey('ArrowLeft', 3, 6)).toBe(2)
  })

  it('jumps to the first and last tab with Home and End', () => {
    expect(tabIndexForKey('Home', 4, 6)).toBe(0)
    expect(tabIndexForKey('End', 1, 6)).toBe(5)
  })

  it('leaves every other key alone', () => {
    expect(tabIndexForKey('Enter', 1, 6)).toBeNull()
    expect(tabIndexForKey('a', 1, 6)).toBeNull()
  })
})
