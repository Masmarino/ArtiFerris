import { shellQuote, singleQuote } from '../../shared/shell-quote'

/** A name starting with `-` would read as an option, so it goes after `--`, quoted. */
function positional(value: string): string {
  return value.startsWith('-') ? `-- ${singleQuote(value)}` : shellQuote(value)
}

export function npmInstallCommand(name: string, registryUrl: string): string {
  const registry = shellQuote(registryUrl)
  return name.startsWith('-')
    ? `npm install --registry ${registry} ${positional(name)}`
    : `npm install ${shellQuote(name)} --registry ${registry}`
}

export function dockerPullCommand(imageReference: string, tag: string | null): string {
  return `docker pull ${positional(`${imageReference}${tag ? ':' + tag : ''}`)}`
}

export function preferredTag(tags: { tag: string }[]): string | null {
  return tags.find((entry) => entry.tag === 'latest')?.tag ?? tags[0]?.tag ?? null
}
