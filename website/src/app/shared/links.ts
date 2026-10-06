export const APP_URL = 'https://app.artiferris.pro'
export const DOCS_URL = `${APP_URL}/docs`
export const GITHUB_URL = 'https://github.com/Masmarino/ArtiFerris'
export const CHANGELOG_URL = `${GITHUB_URL}/blob/main/CHANGELOG.md`
export const README_ROADMAP_URL = `${GITHUB_URL}#feuille-de-route`
export const README_SECURITY_URL = `${GITHUB_URL}#sécurité`
export const README_LICENSE_URL = `${GITHUB_URL}#licence`
export const FERRISGIT_URL = 'https://www.ferrisgit.pro'

// Deep links into the documentation, which is written in French.
export const DOCS = {
  install: `${DOCS_URL}/administration/installation`,
  configuration: `${DOCS_URL}/administration/configuration`,
  organisations: `${DOCS_URL}/administration/organisations-et-utilisateurs`,
  repositories: `${DOCS_URL}/utilisation/depots`,
  npm: `${DOCS_URL}/utilisation/npm`,
  docker: `${DOCS_URL}/utilisation/docker`,
  api: `${DOCS_URL}/api/api-publique`,
} as const
