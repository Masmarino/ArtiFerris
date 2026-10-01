import { InjectionToken } from '@angular/core'
import { Observable } from 'rxjs'
import {
  AdminStats,
  HealthStatus,
  MetricsSnapshot,
  RepositoryUsage,
} from '../domain/metrics.entity'

export interface MetricsPort {
  usage(organizationId?: string): Observable<RepositoryUsage[]>
  health(): Observable<HealthStatus>
  stats(organizationId?: string): Observable<AdminStats>
  history(days?: number): Observable<MetricsSnapshot[]>
}

export const METRICS_PORT = new InjectionToken<MetricsPort>('MetricsPort')
