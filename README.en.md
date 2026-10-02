*[Lire en français](README.md)*

# ArtiFerris

Self-hosted artifact manager: an npm registry and a Docker/OCI registry behind one admin console, one sign-in and the
same controls (quotas, retention, permissions, audit).

A hexagonal Rust backend, with an Angular 22 frontend served by the same binary.

## Contents

- [Features](#features)
- [Architecture](#architecture)
- [Running locally](#running-locally)
- [Deploying](#deploying)
- [Configuration](#configuration)
- [Upgrades and encryption key rotation](#upgrades-and-encryption-key-rotation)
- [Security](#security)
- [Search engines (SEO)](#search-engines-seo)
- [Public API](#public-api)
- [Roadmap](#roadmap)
- [License](#license)

## Features

**Registries**
- npm (publish, install, unpublish, dist-tags) and Docker/OCI (push, pull, manifest deletion).
- Three repository types per format: **hosted** (your content), **proxy** (a cache in front of an upstream registry)
  and **group** (several repositories behind one entry point).
- Storage quota and retention per repository (the last N versions or tags; `latest` is never purged; purge every 6
  hours). An npm package has at most 5,000 versions and 32 MiB of manifests.
- Republishing an unpublished npm version is refused.
- Package and image browser: npm README (converted and sanitized on the server), install command, Docker tag sizes.
- Public catalogs without authentication (`/artiferris-npm`, `/artiferris-docker`), profiles `/@user` and
  `/o/<organization>`. The `artiferris-` prefix is reserved.

**Artifact security**
- npm dependency audit on every publish.
- Docker image scan with [Trivy](https://github.com/aquasecurity/trivy) on every push, re-runnable by hand.

**Multi-tenant**
- One organization per subdomain (`acme.artiferris.example`), with its own repositories, users and branding.
- Organization admins act only on their own; a super-admin targets any with `?organization_id=`.
- A default public organization for single-tenant deployments.

**Accounts and access**
- Local accounts (Argon2), LDAP/Active Directory and OIDC. SAML is not supported.
- Mandatory second factor (TOTP or passkey) with backup codes.
- Personal API tokens; admins can list and revoke everyone's.
- Per-repository permissions (`read`, `write`, `admin`), e-mail invitations (the admin enters only the address, the invitee picks their username on activation), throttling of failed sign-ins.

**Administration**
- Usage metrics and history, health status, audit log and security log.
- Users, SMTP (with a test e-mail), custom branding (logo, favicon).
- Configuration export and import (single-organization instances without personal repositories). The import is one
  transaction: what it cannot restore is listed and skipped, the rest is written as a whole.
- HTML e-mails in the recipient's language (account created, password regenerated, second factor added).

**Languages**: the interface is in French, English, Spanish, Italian and German. The account's language wins over the
browser's; e-mails and public page titles follow the reader's language.

## Architecture

Dependencies point inward.

| Crate | Role |
|---|---|
| `artiferris-domain` | Entities, value objects and ports, with no framework or I/O |
| `artiferris-application` | Use cases |
| `artiferris-infrastructure` | Postgres, files, SMTP, Trivy, Argon2, JWT |
| `artiferris-api` | Axum server, routes, wiring, serves the frontend |
| `artiferris-npm` | npm registry protocol |
| `artiferris-docker` | Docker/OCI registry protocol |

Repositories and permissions are event-sourced; everything else (users, settings, audit, metrics) is plain CRUD on
Postgres. Storage is the local file system (`StorageBackendPort` is abstract, but no other implementation exists).

Stack: Rust (2024 edition), Axum, SQLx, Postgres, webauthn-rs, lettre; Angular 22 (standalone components, signals).

## Running locally

```bash
cp .env.example .env   # set POSTGRES_PASSWORD and JWT_SECRET
./scripts/dev.sh
```

The script starts Postgres (`docker-compose`), applies the migrations, then runs the backend (port 8081) and the
frontend (`ng serve`, port 4200). An `admin` / `admin123` account is created.

```bash
cargo test --workspace        # backend
npm test --prefix frontend    # frontend
```

## Deploying

### Docker Compose

```bash
cp .env.example .env   # POSTGRES_PASSWORD, JWT_SECRET, SECRETS_ENCRYPTION_KEY, PUBLIC_URL, ARTIFERRIS_BOOTSTRAP_ADMIN_*
docker compose up -d --build
```

The server refuses to start if `JWT_SECRET` or `SECRETS_ENCRYPTION_KEY` is under 32 bytes or starts with `change-me`,
if the bootstrap admin password is still the placeholder, or if `PUBLIC_URL` points at `0.0.0.0`. Generate secrets with
`openssl rand -base64 48` (`openssl rand -hex 24` for `POSTGRES_PASSWORD`, which ends up in a URL).

`/readyz` checks that the database answers; `/healthz` only says the process is running.

Measured on a development machine with 15 to 20 clients: 1 core and 256 MB are enough; 0.5 core and 128 MB work, with
a throttled CPU.

### Kubernetes (Helm)

```bash
helm upgrade --install artiferris ./helm/artiferris \
  --namespace artiferris --create-namespace \
  --set image.tag=<release> \
  --set ingress.host=app.example.com --set ingress.wildcardHost='*.example.com' \
  --set artiferris.baseDomain=example.com
```

- `image.tag` has no default; `image.digest` pins the image (CI does).
- The chart generates the database password, `JWT_SECRET` and `SECRETS_ENCRYPTION_KEY` in `<release>-secrets` and finds
  them again with `lookup`. With `helm template`, `--dry-run`, Argo CD or Flux, `lookup` returns nothing: set
  `secrets.postgresPassword`, `secrets.jwtSecret` and `secrets.secretsEncryptionKey`, or every render invents new ones
  and the stored secrets become unreadable.
- The Secret and both volumes (registry and database) survive `helm uninstall` (`persistence.keepOnUninstall`).
- Pods run as uid 100 / gid 101, without privilege escalation, with a read-only file system and the `RuntimeDefault`
  seccomp profile; a NetworkPolicy lets only ArtiFerris reach Postgres.
- The ingress uses the Traefik middlewares of `ingress.middlewares`, which must exist; an empty value uses none.
- With the ingress, set `artiferris.trustedProxyIps` (the controller's pod network): the chart refuses to render
  otherwise, since all visitors would share one sign-in counter (`artiferris.allowSharedThrottleBucket=true` accepts it).
- CI deploys with `--atomic --cleanup-on-fail`. Migrations already applied stay: see the next section to roll back.

## Configuration

Variables read by `artiferris-api`. `docker-compose.yml` wires those of a single-node deployment.

| Variable | Required | Default | Description |
|---|---|---|---|
| `DATABASE_URL` | Yes | | Postgres connection. |
| `JWT_SECRET` | Yes | | Signs sessions and Docker tokens. At least 32 random bytes. Changing it invalidates every session and `docker login`. |
| `SECRETS_ENCRYPTION_KEY` | Yes | | Encrypts stored secrets (SMTP, LDAP, OIDC, proxy credentials, TOTP seeds). Different from `JWT_SECRET`, random (HKDF does not strengthen a passphrase). |
| `SECRETS_ENCRYPTION_KEY_PREVIOUS` | No | | During a rotation: the key the stored secrets currently use. |
| `SECRETS_REENCRYPT_LEGACY` | No | `false` | Allows rewriting stored secrets in the current format and under the new key. |
| `ARTIFERRIS_BASE_DOMAIN` | Yes | | Base domain of organizations (`artiferris.example` for `acme.artiferris.example`). No default: a config mistake must fail at startup, not route every subdomain to the public organization. |
| `PUBLIC_URL` | Except locally | derived from `BIND_ADDR` | External URL: invitation links, HSTS and Docker realm scheme, passkey origin, canonical URLs. |
| `ARTIFERRIS_BOOTSTRAP_ADMIN_USERNAME` / `_PASSWORD` | A first admin is needed | | Account created only when the `users` table is empty. Password of at least 8 characters, not `change-me…`. |
| `ARTIFERRIS_SSRF_ALLOWED_CIDRS` | No | | Ranges (`10.20.0.0/16,192.168.1.5`) that LDAP, SMTP, OIDC and proxies may reach although private. Otherwise private, loopback and link-local addresses are refused. Unencrypted SMTP is accepted only for these hosts. A `/0` range or a typo stops the server. |
| `TRUSTED_PROXY_IPS` | No | | Reverse proxies whose `X-Forwarded-For` is believed (CIDR ranges). A `/0` range stops the server. |
| `ANONYMOUS_REGISTRY_READS_PER_MINUTE` | No | `1200` | Anonymous npm and Docker reads per minute and client; `0` removes the limit. |
| `DB_MAX_CONNECTIONS` | No | `10` | Pool size. |
| `ARTIFERRIS_AUDIT_BACKFILL_FORCE` | No | `false` | The task that fills in the organization of old audit events does not start under 3 connections; `true` forces it. |
| `STORAGE_ROOT` | No | `./data` | npm tarballs and Docker blobs. A persistent volume in production. |
| `BIND_ADDR` | No | `0.0.0.0:8080` | Listen address. |
| `STATIC_DIR` | No | `./static` | Built frontend (outside the provided image). |
| `RUST_LOG` | No | no logs | `tracing` filter, for example `info`. |
| `CORS_ALLOWED_ORIGIN` | No | any origin | Restricts CORS to one origin. Leave empty when the API serves the frontend. |
| `ARTIFERRIS_DOCKER_TOKEN_REALM` | No | derived from the request | Docker token realm, normally computed per request. Set it only behind an intermediary that does not forward `Host` and the scheme faithfully; every client is then pinned to that realm. `https://` outside localhost. |

Not configurable (hard-coded): metrics purge (hourly), retention purge (6 hours) and abandoned Docker uploads (hourly,
unreferenced blobs after 48 hours). The Docker scan runs `trivy`, present in the provided image.

## Upgrades and encryption key rotation

Stored secrets are written in a versioned format that earlier versions cannot read. Existing values are rewritten only
if `SECRETS_REENCRYPT_LEGACY=true` (Helm: `artiferris.reencryptStoredSecrets`); without it, the server logs how many
values are waiting.

**To upgrade**: back up the database, deploy without `SECRETS_REENCRYPT_LEGACY`, check SSO, e-mail and MFA, then set it
to `true` and restart. The `re-encrypted stored secrets` line gives the number of values converted.

**Rolling back** means restoring the database: this version applies migrations 0006 to 0010, and the previous one
(0.4.6 and earlier) refuses to start on a database that records migrations it does not know. `helm rollback` and
`--atomic` do not undo them. From this version on, the server ignores migrations of a newer version, which only works
if that schema change was additive (the release notes say so). Earlier versions also cannot read the new secret
format: TOTP, SMTP, LDAP, OIDC and proxies stop working.

**To change the key**:
1. Back up the database.
2. Put the new value in `SECRETS_ENCRYPTION_KEY`, the old one in `SECRETS_ENCRYPTION_KEY_PREVIOUS` and set
   `SECRETS_REENCRYPT_LEGACY=true`. With Helm: `--set secrets.secretsEncryptionKey=<new> --set
   secrets.secretsEncryptionKeyPrevious=<old> --set artiferris.reencryptStoredSecrets=true`.
3. Restart all replicas together: a replica on the old key cannot read what another has moved.
4. `re-encrypted stored secrets` confirms; an error line flags values unreadable with both keys, left as they are.
5. Remove `SECRETS_ENCRYPTION_KEY_PREVIOUS` (Helm: remove `secretsEncryptionKeyPrevious` and upgrade again).

A value that cannot be read (wrong key, after a restore) is logged with its type and owner, and the SSO and SMTP
settings return `secret_unreadable: true`. Whatever depends on it stays down until the right key is back or the secret
is typed again.

## Security

- Argon2 passwords; mandatory MFA; a separate, short-lived token for "password verified, second factor pending", not
  interchangeable with a session.
- Per-repository permissions checked on every route, not just hidden in the interface.
- Audit log (365 days by default, `AUDIT_RETENTION_DAYS`, `0` to keep everything) and security log. Failed sign-ins
  record the typed name (64 characters at most): a password pasted into that field may end up there, readable by
  super-admins.
- Request bodies are read only after the repository, role and scope checks. Docker blobs go to disk as they arrive;
  documents held in memory draw on a 1 GiB budget (`503` when spent), and one client or user gets four bodies in
  flight (`429`).
- Docker access tokens last 2 minutes. Reading through a group needs `read` on every member.
- Public content is cached for 1 year by digest (Docker blobs and manifests) and 1 day for npm tarballs; private
  content is `private, no-store`. A CDN may keep a blob after a repository goes private: put the registry behind a
  cache that honors purges, or do not cache `/v2/` at the edge.
- `/api` responses are `no-store` and `Vary: Authorization`. Every response carries `Permissions-Policy` and
  `Cross-Origin-Opener-Policy: same-origin`.
- A JSON request whose body stalls for 30 s or runs over 120 s gets `408`. Connections and edge timeouts are the
  ingress's job.
- `npm audit` from the interface: 30 runs per minute per account, results kept for 10 minutes. The dependency scan
  sends package names and versions to `registry.npmjs.org`; do not point `npm audit` at ArtiFerris if those names must
  not leave your network.
- A proxy sends its credentials only to the configured remote (same scheme, host and port, over `https`); elsewhere it
  fetches anonymously.
- Downloads are counted per client (IPv4, IPv6 /64) once per package per hour, in memory. Behind a reverse proxy, set
  `TRUSTED_PROXY_IPS`.
- Anonymous traffic has a per-client (IPv4, IPv6 /64) budget per minute: public pages and API, npm and Docker reads
  (`ANONYMOUS_REGISTRY_READS_PER_MINUTE`, 1,200 by default, `0` removes the limit; `429` with `Retry-After`). A request
  carrying a token is never counted. Replicas share these counters through the database every two seconds: a client can
  exceed the limit by at most what arrives between two syncs, and if the database does not answer each replica limits on
  its own. Postgres only receives keyed hashes, never an address. This budget does not stop a distributed flood: put a
  limit at the ingress or a CDN in front. Failed sign-ins have their own limit, per process.
- Branding files are validated by their signature, never by their `Content-Type`.
- The activation token travels in the URL (`/activate?token=…`); it is single-use on the server and expires. When an
  SSO sign-in comes back, the browser only accepts the session token if it started that sign-in in the last 10 minutes,
  on top of the cookie and `state` the server checks.

Package READMEs are sanitized on the server, but their images may come from any `https` host: on public pages each
visitor therefore reveals their IP and User-Agent to that host. This is an accepted risk; to reduce it, restrict
`img-src` in `crates/artiferris-api/src/main.rs`.

Found a vulnerability? Report it privately, without opening a public issue.

## Search engines (SEO)

The public catalog pages (`/explorer`, `/artiferris-npm`, `/artiferris-docker`, `/@user`, `/o/<organization>` and their
repositories, packages and images) have their own `<head>`: title, description, canonical URL, Open Graph, Twitter
card and schema.org data for packages. The body is still rendered by the browser.

**Indexing is off by default.** A super-admin turns it on in Administration, settings of the public organization. Off,
every page sends `noindex`, `robots.txt` disallows everything and the sitemap is empty. On, `/robots.txt` disallows the
API, the registries and the app, and `/sitemap.xml` (files of 40,000 URLs, cached 10 minutes, 500,000 URLs at most)
lists the public pages. Search pages (`?q=`) stay `noindex`.

**Language**: that of the `Accept-Language` header (the first translated language in order of preference; `fr-CA`
gives French), English otherwise, so for a crawler. The URL is the same in every language: the canonical tag does not
change, the response carries `Vary: Accept-Language`, and `<html lang>`, `og:locale` and `inLanguage` follow.

**Blocking, per organization and for the instance** (Administration, System settings):
- "Public page": closed, the organization's pages answer as if nothing was published (absent from search, suggestions,
  counts and the sitemap). On the public organization it closes the whole instance's catalog (super-admin):
  `robots.txt` disallows everything and the sitemap is empty.
- "Keep search engines away": the pages stay visible but send `noindex, nofollow` and leave the sitemap.

Both are open by default. The instance's indexing switch stays the master. Personal repositories (`/@user`) follow the
instance's settings.

The sitemap is rebuilt by one request at a time (the previous copy serves meanwhile, and if a rebuild fails). The
indexing setting is re-read every 30 seconds at most; with no known value, `robots.txt` and the sitemap answer `503`. A
`<head>` that takes over 300 ms is replaced by the generic one. Canonical URLs come from `PUBLIC_URL`. A private,
unknown or deleted repository gets the generic `<head>`, revealing nothing.

## Public API

**Authentication**
- `POST /api/auth/login`: credentials, then an MFA enrollment response.
- `POST /api/auth/register`: self-registration in the public organization (400 elsewhere), same response.

**Package and image details** (60 requests per minute per IP for an anonymous caller, `429` beyond)
- `GET /api/repositories/{id}/packages/npm/{name}`: versions, dist-tags, `downloads_7d`, `readme_html` (128 KB read,
  sanitized; `null` without a README) and `registry_url`.
- `GET /api/repositories/{id}/packages/docker/{image}`: `downloads_7d`, tags (`size_bytes`, `null` for a
  multi-architecture index) and `image_reference`.

Downloads count an npm tarball `GET` or a Docker manifest `GET` by tag on a hosted repository (not `HEAD`, digest,
proxy or group), written every 30 seconds, per day, with no user data, purged after 13 months. The figures are
indicative.

**Public catalog**: no authentication, `Cache-Control: no-store`, 60 requests per minute per IP (`429` with
`Retry-After`). The landing lists and counts are cached 30 seconds: a repository made private can stay listed that
long, its content remaining protected.
- `GET /api/public/catalogs`: one catalog per format (`format`, `name`, `label`, `entry_count`).
- `GET /api/public/search`: parameters `q` (100 characters at most), `format` (`npm`, `docker`), `owner`
  (`personal:<user>`, `organization:<slug>`), `sort` (`relevance`, `updated`, `popular`), `page` (1 to 100), `per_page`
  (1 to 50, 20 by default). Tolerates typos from 3 characters (`match_kind: fuzzy`, last). Each result carries
  `downloads_7d`, the owner, the repository and the install URL.
- `GET /api/search`: the same search for a signed-in user, over everything they can read (proxies included, with what
  they have cached); each `repository` adds `id` and `repo_type`. A super-admin searches everywhere. `401` without a
  token.
- `GET /api/public/suggest`: `q` of 2 to 100 characters, 8 results at most (exact name, prefix, substring, then
  approximate); filters `format`, `owner` and `limit`. Its own budget: 120 requests per minute per IP.
- `GET /api/public/owners/{kind}/{slug}`: name and counts of repositories, packages and images. `404` without a public
  repository, the same answer as for an unknown owner.
- `GET /api/repositories/by-org/{slug}/{repo_name}`: a public organization repository (`404` otherwise), the
  counterpart of `by-owner`. An anonymous caller gets it without `quota_bytes`, `retention_keep_last_n` or
  `organization_id`.

## Roadmap

- [ ] SAML.
- [ ] More formats: Maven/Gradle, PyPI, NuGet, Cargo, Go, Helm, raw repositories.
- [ ] S3-compatible object storage behind `StorageBackendPort`, for several replicas (today one volume, one replica).
- [ ] High availability and geo-replication.
- [ ] Package signing and provenance (Sigstore, npm provenance).
- [ ] Per-user namespaces inside an organization (`@user/…` scopes).
- [ ] Rate limiting of anonymous traffic; CDN distribution.
- [x] Public package search and discovery.

## License

No license file is included: the code is all rights reserved until there is one.
