import { inject } from '@angular/core'
import { DOCUMENT } from '@angular/common'

const SVG_NS = 'http://www.w3.org/2000/svg'

/**
 * domino, the DOM behind the prerender, throws "NotYetImplemented" when innerHTML is set on an SVG element, which is
 * what Gabarit's gbt-icon does. Without this patch the icons only show up once JavaScript runs.
 * The setter for HTML elements parses SVG fragments fine, so SVG elements borrow it. Server only.
 */
export function installSvgInnerHtml(): void {
  const doc = inject(DOCUMENT)
  const svg = doc.createElementNS(SVG_NS, 'svg')

  let htmlProto: object | null = Object.getPrototypeOf(doc.createElement('div'))
  while (htmlProto && !Object.prototype.hasOwnProperty.call(htmlProto, 'innerHTML')) {
    htmlProto = Object.getPrototypeOf(htmlProto)
  }
  const descriptor = htmlProto && Object.getOwnPropertyDescriptor(htmlProto, 'innerHTML')
  if (!descriptor?.set) return

  Object.defineProperty(Object.getPrototypeOf(svg), 'innerHTML', {
    configurable: true,
    get: descriptor.get,
    set: descriptor.set,
  })
}
