import { Lang } from '../i18n/languages'

/** Where each of the seven circles of the plan leads: the numbered tiles of the home page. */
export const PLAN_FRAGMENTS: readonly string[] = ['d1', 'd2', 'd3', 'd4', 'd5', 'd6', 'd7']

/** Router commands for the page that carries the plan, in a language. */
export function planPageCommands(lang: Lang): string[] {
  return ['/', lang]
}
