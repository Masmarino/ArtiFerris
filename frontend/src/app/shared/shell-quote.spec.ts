import { shellQuote } from './shell-quote'

describe('shellQuote', () => {
  it.each(['left-pad', '@alice/button', 'localhost:4200/u/alice/repo/hello:v1', 'a.b_c+d'])(
    'leaves %s alone',
    (value) => {
      expect(shellQuote(value)).toBe(value)
    },
  )

  it.each([
    ['a b', "'a b'"],
    ['a;rm -rf ~', "'a;rm -rf ~'"],
    ['$(id)', "'$(id)'"],
    ['a`id`', "'a`id`'"],
    ['a&b|c>d', "'a&b|c>d'"],
    ['line\nbreak', "'line\nbreak'"],
    ['', "''"],
  ])('quotes %j', (value, expected) => {
    expect(shellQuote(value)).toBe(expected)
  })

  it('escapes a single quote inside the value', () => {
    expect(shellQuote("it's")).toBe(`'it'\\''s'`)
  })
})
