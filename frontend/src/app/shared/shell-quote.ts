const PLAIN_WORD = /^[\w@:./+-]+$/

export function singleQuote(value: string): string {
  return `'${value.replace(/'/g, `'\\''`)}'`
}

export function shellQuote(value: string): string {
  return PLAIN_WORD.test(value) ? value : singleQuote(value)
}
