import { Pipe, PipeTransform } from '@angular/core'
import type { SelectOption } from '@masmarino/gabarit'

// Case-insensitive so npm's lowercase and Trivy's uppercase severities sort the same way. "medium" and "moderate" are the same tier under different vocabularies.
const SEVERITY_RANK: Record<string, number> = {
  critical: 0,
  high: 1,
  medium: 2,
  moderate: 2,
  low: 3,
  unknown: 4,
}

export function severityRank(severity: string): number {
  return SEVERITY_RANK[severity.toLowerCase()] ?? Number.MAX_SAFE_INTEGER
}

export function bySeverityDesc<T>(getSeverity: (item: T) => string): (a: T, b: T) => number {
  return (a, b) => severityRank(getSeverity(a)) - severityRank(getSeverity(b))
}

@Pipe({ name: 'severityClass' })
export class SeverityClassPipe implements PipeTransform {
  transform(severity: string): string {
    const normalized = severity.toLowerCase()
    if (normalized === 'critical' || normalized === 'high') {
      return 'package-detail__severity--high'
    }
    if (normalized === 'moderate' || normalized === 'medium') {
      return 'package-detail__severity--moderate'
    }
    return 'package-detail__severity--low'
  }
}

export const DOCKER_SEVERITY_OPTIONS: SelectOption<string>[] = [
  { value: 'CRITICAL', label: 'Critique' },
  { value: 'HIGH', label: 'Élevée' },
  { value: 'MEDIUM', label: 'Moyenne' },
  { value: 'LOW', label: 'Faible' },
  { value: 'UNKNOWN', label: 'Inconnue' },
]

export const NPM_SEVERITY_OPTIONS: SelectOption<string>[] = [
  { value: 'critical', label: 'Critique' },
  { value: 'high', label: 'Élevée' },
  { value: 'moderate', label: 'Moyenne' },
  { value: 'low', label: 'Faible' },
]
