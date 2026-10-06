import { DefaultUrlSerializer, UrlTree } from '@angular/router'

/**
 * Serialises URLs with a trailing slash (/fr/features/). The prerender writes fr/features/index.html and nginx serves
 * it at that address; without the slash every link would cost a redirect and differ from the canonical URL.
 * Parsing is unchanged: both forms match the same route.
 */
export class TrailingSlashUrlSerializer extends DefaultUrlSerializer {
  override serialize(tree: UrlTree): string {
    const url = super.serialize(tree)
    const end = url.search(/[?#]/)
    const path = end === -1 ? url : url.slice(0, end)
    // A file name (/404.html, /robots.txt) isn't a directory, so it keeps its form.
    if (path.endsWith('/') || /\.[^/]*$/.test(path)) return url
    return `${path}/${end === -1 ? '' : url.slice(end)}`
  }
}
