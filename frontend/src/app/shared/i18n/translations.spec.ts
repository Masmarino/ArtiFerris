import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'
import fr from '../../../../public/i18n/fr.json'

const SOURCE_ROOT = join(process.cwd(), 'src', 'app')

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name)
    if (statSync(path).isDirectory()) {
      return sourceFiles(path)
    }
    const isSource = /\.(ts|html)$/.test(name) && !/\.(spec|stories)\.ts$/.test(name)
    return isSource ? [path] : []
  })
}

function leafKeys(node: unknown, prefix = ''): string[] {
  if (typeof node === 'string') {
    return [prefix]
  }
  return Object.entries(node as Record<string, unknown>).flatMap(([key, child]) =>
    leafKeys(child, prefix ? `${prefix}.${key}` : key),
  )
}

const TOP_LEVEL = Object.keys(fr)
const KEY_LITERAL = new RegExp(`['"\`]((?:${TOP_LEVEL.join('|')})(?:\\.[\\w]+)+)['"\`]`, 'g')
// Keys assembled at runtime (`t(\`admin.audit.events.${type}\`)`), which no literal can reveal.
const DYNAMIC_PREFIXES = [
  'admin.audit.events.',
  'admin.audit.loginMethods.',
  'admin.audit.mfaMethods.',
  'admin.audit.settings.',
  'admin.audit.details.logo',
  'admin.audit.details.favicon',
  'repositories.vulnerability.severity.',
  'format.bytes.',
  // `errors.api.<code>`, one per error code the API can send (checked below).
  'errors.api.',
]

const sources = sourceFiles(SOURCE_ROOT).map((path) => readFileSync(path, 'utf8'))
const referenced = new Set(
  sources.flatMap((source) => [...source.matchAll(KEY_LITERAL)].map((match) => match[1])),
)
const defined = new Set(leafKeys(fr))
const plural = /_(one|other)$/

/** Every error code the API can send: the ones of the two error enums, and the ones a route sets itself. */
function apiErrorCodes(): string[] {
  const crates = join(process.cwd(), '..', 'crates')
  const enumCodes = [
    join(crates, 'artiferris-domain', 'src', 'error.rs'),
    join(crates, 'artiferris-application', 'src', 'error.rs'),
  ].flatMap((file) =>
    [...readFileSync(file, 'utf8').matchAll(/=> "([a-z_]+)",/g)].map((match) => match[1]),
  )
  const routeCodes = rustFiles(join(crates, 'artiferris-api', 'src')).flatMap((file) =>
    [...readFileSync(file, 'utf8').matchAll(/ErrorResponse::coded\("([a-z_]+)"/g)].map(
      (match) => match[1],
    ),
  )
  return [...new Set([...enumCodes, ...routeCodes])]
}

function rustFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name)
    if (statSync(path).isDirectory()) {
      return rustFiles(path)
    }
    return name.endsWith('.rs') ? [path] : []
  })
}

describe('translations (fr.json)', () => {
  it('has a message for every error code the API can send', () => {
    const codes = apiErrorCodes()

    expect(codes.length).toBeGreaterThan(50)
    expect(codes.filter((code) => !defined.has(`errors.api.${code}`))).toEqual([])
    expect(
      [...defined]
        .filter((key) => key.startsWith('errors.api.'))
        .filter((key) => !codes.includes(key.slice('errors.api.'.length))),
    ).toEqual([])
  })

  it('defines every key the code refers to', () => {
    const missing = [...referenced].filter(
      (key) =>
        !defined.has(key) &&
        !defined.has(`${key}_one`) &&
        !DYNAMIC_PREFIXES.some((prefix) => key.startsWith(prefix)),
    )

    expect(missing).toEqual([])
  })

  it('has no key that nothing refers to', () => {
    const unused = [...defined].filter((key) => {
      const base = key.replace(plural, '')
      return (
        !referenced.has(key) &&
        !referenced.has(base) &&
        !DYNAMIC_PREFIXES.some((prefix) => key.startsWith(prefix))
      )
    })

    expect(unused).toEqual([])
  })
})
