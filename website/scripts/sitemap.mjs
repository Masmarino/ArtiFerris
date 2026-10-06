// Pure sitemap builder, with no file access so that the unit tests can call it. build-sitemap.mjs does the I/O.
// The types are in sitemap.d.mts.

export function pageUrl(site, lang, path) {
  return `${site.siteUrl}/${lang}/${path ? `${path}/` : ''}`
}

export function pageUrls(site) {
  return Object.values(site.pages).flatMap((path) =>
    site.langs.map((lang) => pageUrl(site, lang, path)),
  )
}

function escapeXml(value) {
  return value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

// One <url> per page and language. Each lists every language of the page, itself included as Google requires, plus
// x-default for the default language.
export function buildSitemap(site) {
  const entries = []
  for (const path of Object.values(site.pages)) {
    const alternates = [
      ...site.langs.map((lang) => ({ hreflang: lang, href: pageUrl(site, lang, path) })),
      { hreflang: 'x-default', href: pageUrl(site, site.defaultLang, path) },
    ]
      .map(
        ({ hreflang, href }) =>
          `    <xhtml:link rel="alternate" hreflang="${hreflang}" href="${escapeXml(href)}"/>`,
      )
      .join('\n')
    for (const lang of site.langs) {
      const loc = escapeXml(pageUrl(site, lang, path))
      entries.push(`  <url>\n    <loc>${loc}</loc>\n${alternates}\n  </url>`)
    }
  }
  return `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">
${entries.join('\n')}
</urlset>
`
}
