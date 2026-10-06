export interface SiteConfig {
  siteUrl: string
  langs: string[]
  defaultLang: string
  pages: Record<string, string>
}

export function pageUrl(site: SiteConfig, lang: string, path: string): string
export function pageUrls(site: SiteConfig): string[]
export function buildSitemap(site: SiteConfig): string
