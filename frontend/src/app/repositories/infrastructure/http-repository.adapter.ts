import { Injectable, inject } from '@angular/core'
import { HttpClient, HttpParams } from '@angular/common/http'
import { Observable } from 'rxjs'
import {
  CreateRepositoryOptions,
  DockerImageDetails,
  DockerImageScanResult,
  NpmAdvisory,
  NpmDependencyAuditResult,
  NpmPackageDetails,
  RepositoryFormat,
  RepositoryPackages,
  RepositorySummary,
  RepositoryType,
} from '../domain/repository.entity'
import { RepositoryPort } from '../application/repository.port'
import { apiPath } from '../../shared/api-path'

@Injectable()
export class HttpRepositoryAdapter implements RepositoryPort {
  private readonly http = inject(HttpClient)

  list(): Observable<RepositorySummary[]> {
    return this.http.get<RepositorySummary[]>('/api/repositories')
  }

  get(id: string): Observable<RepositorySummary> {
    return this.http.get<RepositorySummary>(apiPath`/api/repositories/${id}`)
  }

  getByOwner(username: string, repoName: string): Observable<RepositorySummary> {
    return this.http.get<RepositorySummary>(
      apiPath`/api/repositories/by-owner/${username}/${repoName}`,
    )
  }

  getByOrg(slug: string, repoName: string): Observable<RepositorySummary> {
    return this.http.get<RepositorySummary>(apiPath`/api/repositories/by-org/${slug}/${repoName}`)
  }

  create(
    name: string,
    format: RepositoryFormat,
    repoType: RepositoryType,
    remoteUrl: string | null,
    options: CreateRepositoryOptions = {},
  ): Observable<RepositorySummary> {
    return this.http.post<RepositorySummary>('/api/repositories', {
      name,
      format,
      repo_type: repoType,
      remote_url: remoteUrl,
      remote_username: options.remoteUsername ?? null,
      remote_password: options.remotePassword ?? null,
      group_members: options.groupMembers ?? [],
      quota_bytes: options.quotaBytes ?? null,
      retention_keep_last_n: options.retentionKeepLastN ?? null,
    })
  }

  rename(id: string, name: string): Observable<void> {
    return this.http.patch<void>(apiPath`/api/repositories/${id}`, { name })
  }

  setQuota(id: string, quotaBytes: number | null): Observable<void> {
    return this.http.put<void>(apiPath`/api/repositories/${id}/quota`, { quota_bytes: quotaBytes })
  }

  setRetentionPolicy(id: string, keepLastN: number | null): Observable<void> {
    return this.http.put<void>(apiPath`/api/repositories/${id}/retention`, {
      keep_last_n_versions: keepLastN,
    })
  }

  setVisibility(id: string, isPublic: boolean): Observable<void> {
    return this.http.put<void>(apiPath`/api/repositories/${id}/visibility`, { is_public: isPublic })
  }

  delete(id: string): Observable<void> {
    return this.http.delete<void>(apiPath`/api/repositories/${id}`)
  }

  addGroupMember(groupId: string, memberRepositoryId: string, position: number): Observable<void> {
    return this.http.post<void>(apiPath`/api/repositories/${groupId}/group-members`, {
      member_repository_id: memberRepositoryId,
      position,
    })
  }

  removeGroupMember(groupId: string, memberId: string): Observable<void> {
    return this.http.delete<void>(apiPath`/api/repositories/${groupId}/group-members/${memberId}`)
  }

  packages(id: string, after?: string | null): Observable<RepositoryPackages> {
    const params = after ? new HttpParams().set('after', after) : new HttpParams()
    return this.http.get<RepositoryPackages>(apiPath`/api/repositories/${id}/packages`, { params })
  }

  npmPackageDetails(id: string, name: string): Observable<NpmPackageDetails> {
    return this.http.get<NpmPackageDetails>(apiPath`/api/repositories/${id}/packages/npm/${name}`)
  }

  deleteNpmPackage(id: string, name: string): Observable<void> {
    return this.http.delete<void>(apiPath`/api/repositories/${id}/packages/npm/${name}`)
  }

  deleteNpmPackageVersion(id: string, name: string, version: string): Observable<void> {
    return this.http.delete<void>(
      apiPath`/api/repositories/${id}/packages/npm/${name}/versions/${version}`,
    )
  }

  npmPackageAudit(id: string, name: string): Observable<NpmAdvisory[]> {
    return this.http.get<NpmAdvisory[]>(apiPath`/api/repositories/${id}/packages/npm/${name}/audit`)
  }

  getDependencyAudit(
    id: string,
    name: string,
    version: string,
  ): Observable<NpmDependencyAuditResult | null> {
    return this.http.get<NpmDependencyAuditResult | null>(
      apiPath`/api/repositories/${id}/packages/npm/${name}/versions/${version}/dependency-audit`,
    )
  }

  scanDependencyTree(
    id: string,
    name: string,
    version: string,
  ): Observable<NpmDependencyAuditResult> {
    return this.http.post<NpmDependencyAuditResult>(
      apiPath`/api/repositories/${id}/packages/npm/${name}/versions/${version}/dependency-audit`,
      {},
    )
  }

  dockerImageDetails(id: string, imageName: string): Observable<DockerImageDetails> {
    return this.http.get<DockerImageDetails>(
      apiPath`/api/repositories/${id}/packages/docker/${imageName}`,
    )
  }

  deleteDockerImage(id: string, imageName: string): Observable<void> {
    return this.http.delete<void>(apiPath`/api/repositories/${id}/packages/docker/${imageName}`)
  }

  deleteDockerTag(id: string, imageName: string, tag: string): Observable<void> {
    return this.http.delete<void>(
      apiPath`/api/repositories/${id}/packages/docker/${imageName}/tags/${tag}`,
    )
  }

  getDockerImageScan(
    id: string,
    imageName: string,
    tag: string,
  ): Observable<DockerImageScanResult | null> {
    return this.http.get<DockerImageScanResult | null>(
      apiPath`/api/repositories/${id}/packages/docker/${imageName}/tags/${tag}/scan`,
    )
  }

  scanDockerImage(id: string, imageName: string, tag: string): Observable<DockerImageScanResult> {
    return this.http.post<DockerImageScanResult>(
      apiPath`/api/repositories/${id}/packages/docker/${imageName}/tags/${tag}/scan`,
      {},
    )
  }
}
