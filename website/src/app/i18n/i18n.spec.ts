import { LANGS } from './languages'
import de from './de.json'
import en from './en.json'
import es from './es.json'
import fr from './fr.json'
import itDictionary from './it.json'

const DICTIONARIES: Record<string, unknown> = { en, fr, it: itDictionary, es, de }

// Flattens { a: { b: 'x' } } into { 'a.b': 'x' }.
function flatten(value: unknown, prefix = ''): Record<string, unknown> {
  if (typeof value !== 'object' || value === null) return { [prefix]: value }
  return Object.entries(value).reduce<Record<string, unknown>>(
    (all, [key, child]) => ({ ...all, ...flatten(child, prefix ? `${prefix}.${key}` : key) }),
    {},
  )
}

const reference = flatten(en)

describe('translations', () => {
  it('cover the five languages of the site', () => {
    expect(Object.keys(DICTIONARIES).sort()).toEqual([...LANGS].sort())
  })

  for (const lang of LANGS) {
    describe(lang, () => {
      const entries = flatten(DICTIONARIES[lang])

      it('has exactly the keys of en.json', () => {
        const missing = Object.keys(reference).filter((key) => !(key in entries))
        const extra = Object.keys(entries).filter((key) => !(key in reference))
        expect({ missing, extra }).toEqual({ missing: [], extra: [] })
      })

      it('has no empty value and no TODO marker', () => {
        const bad = Object.entries(entries)
          .filter(
            ([, value]) =>
              typeof value !== 'string' || value.trim() === '' || /\bTODO\b/.test(value),
          )
          .map(([key]) => key)
        expect(bad).toEqual([])
      })

      it('closes every inline code span and leaves no template syntax behind', () => {
        const bad = Object.entries(entries)
          .filter(([, value]) => {
            const text = String(value)
            return (text.match(/`/g) ?? []).length % 2 !== 0 || /\{\{|\}\}|<[a-z/]/i.test(text)
          })
          .map(([key]) => key)
        expect(bad).toEqual([])
      })
    })
  }

  it('keeps technical names untouched in every language', () => {
    for (const lang of LANGS) {
      const entries = flatten(DICTIONARIES[lang])
      expect(entries['home.hero.title']).toBeDefined()
      expect(String(entries['features.sections.tech.bullets.b3'])).toContain('masmarino/artiferris')
      expect(String(entries['roadmap.groups.scale.intro'])).toContain('StorageBackendPort')
      expect(String(entries['registries.noteCold'])).toContain('registry.npmjs.org')
    }
  })

  it('keeps the plan labels and the scene labels short: SVG text does not wrap, the window is narrow', () => {
    for (const lang of LANGS) {
      const entries = flatten(DICTIONARIES[lang])
      const long = Object.entries(entries)
        .filter(([key]) =>
          /^plan\.[a-z]+\.(l\d|name)$|^scene\.(open|merged|running|passed|apply|applied)$/.test(
            key,
          ),
        )
        .filter(([, value]) => String(value).length > 31)
        .map(([key, value]) => `${lang} ${key}: ${value}`)
      expect(long).toEqual([])
    }
  })

  it('does not fall back on the vocabulary of a product page, in any language', () => {
    const banned =
      /seamless|powerful|robust|modern|everything you need|solution(?! de repli)|écosystème|ecosystem|enterprise-grade|blazing|next-generation|best-in-class|all-in-one|tout-en-un|tout ce dont vous avez besoin|puissant|leistungsstark|nahtlos|alles, was Sie brauchen|potente|senza soluzione|tutto ciò di cui hai bisogno|todo lo que necesitas|sin fisuras|robusto|moderno|moderne/i
    for (const lang of LANGS) {
      const hits = Object.entries(flatten(DICTIONARIES[lang]))
        .filter(([, value]) => banned.test(String(value)))
        .map(([key]) => `${lang} ${key}`)
      expect(hits).toEqual([])
    }
  })
})
