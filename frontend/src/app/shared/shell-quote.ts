const PLAIN_WORD = /^[\w@:./+-]+$/

export function singleQuote(value: string): string {
  return `'${value.replace(/'/g, `'\\''`)}'`
}

/** Leaves a plain word as is and single-quotes anything else, so a pasted command cannot be reshaped. */
export function shellQuote(value: string): string {
  return PLAIN_WORD.test(value) ? value : singleQuote(value)
}
