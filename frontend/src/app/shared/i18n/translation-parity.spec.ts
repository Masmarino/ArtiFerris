import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'

const I18N_DIR = join(process.cwd(), 'public', 'i18n')
const REFERENCE = 'fr'

function load(lang: string): Record<string, string> {
  const flat: Record<string, string> = {}
  const walk = (node: unknown, prefix: string): void => {
    for (const [key, value] of Object.entries(node as Record<string, unknown>)) {
      const path = prefix ? `${prefix}.${key}` : key
      if (typeof value === 'string') {
        flat[path] = value
      } else {
        walk(value, path)
      }
    }
  }
  walk(JSON.parse(readFileSync(join(I18N_DIR, `${lang}.json`), 'utf8')), '')
  return flat
}

const placeholders = (text: string): string[] =>
  (text.match(/\{\{\s*\w+\s*\}\}/g) ?? []).map((p) => p.replace(/\s+/g, '')).sort()

const reference = load(REFERENCE)
const languages = readdirSync(I18N_DIR)
  .filter((file) => file.endsWith('.json'))
  .map((file) => file.replace(/\.json$/, ''))
  .filter((lang) => lang !== REFERENCE)

describe.each(languages)('translation "%s" against the reference (fr)', (lang) => {
  const translation = load(lang)

  it('has exactly the reference keys', () => {
    const referenceKeys = Object.keys(reference)
    const keys = Object.keys(translation)

    expect(referenceKeys.filter((key) => !keys.includes(key))).toEqual([])
    expect(keys.filter((key) => !referenceKeys.includes(key))).toEqual([])
  })

  it('keeps the same interpolation parameters as the reference', () => {
    const mismatched = Object.keys(reference).filter(
      (key) =>
        key in translation &&
        placeholders(translation[key]).join() !== placeholders(reference[key]).join(),
    )

    expect(mismatched).toEqual([])
  })

  it('leaves no value empty', () => {
    expect(Object.keys(translation).filter((key) => translation[key].trim() === '')).toEqual([])
  })
})
