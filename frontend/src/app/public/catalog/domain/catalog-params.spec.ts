import { parsePage } from './catalog-params'

describe('parsePage', () => {
  it.each([
    ['1', 1],
    ['2', 2],
    ['4294967295', 4294967295],
  ])('reads %j as %d', (value, expected) => {
    expect(parsePage(value)).toBe(expected)
  })

  it.each([
    null,
    '',
    '0',
    '-3',
    '2.5',
    'abc',
    '1e19',
    '1e3',
    '0x10',
    ' 2',
    '4294967296',
    '99999999999',
  ])('falls back to page 1 for %j', (value) => {
    expect(parsePage(value)).toBe(1)
  })
})
