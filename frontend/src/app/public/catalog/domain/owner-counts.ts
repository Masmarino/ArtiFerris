import { OwnerSummary } from './catalog.entity'

function count(value: number, singular: string): string {
  return `${value} ${singular}${value > 1 ? 's' : ''}`
}

/** "2 dépôts · 3 paquets · 4 images", leaving out whatever is zero. */
export function ownerCountsLabel(summary: OwnerSummary): string {
  return [
    summary.repository_count > 0 && count(summary.repository_count, 'dépôt'),
    summary.package_count > 0 && count(summary.package_count, 'paquet'),
    summary.image_count > 0 && count(summary.image_count, 'image'),
  ]
    .filter(Boolean)
    .join(' · ')
}
