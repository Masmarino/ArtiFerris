import { t } from '../../../shared/i18n/translator'
import { OwnerSummary } from './catalog.entity'

function count(value: number, key: string): string {
  return t(value > 1 ? `${key}_other` : `${key}_one`, { count: value })
}

export function ownerCountsLabel(summary: OwnerSummary): string {
  return [
    summary.repository_count > 0 && count(summary.repository_count, 'catalog.counts.repository'),
    summary.package_count > 0 && count(summary.package_count, 'catalog.counts.package'),
    summary.image_count > 0 && count(summary.image_count, 'catalog.counts.image'),
  ]
    .filter(Boolean)
    .join(' · ')
}
