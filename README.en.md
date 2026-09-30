*[Lire en français](README.md)*

# ArtiFerris

Self-hosted artifact repository manager — an npm registry and a Docker/OCI
registry behind one admin console, one auth system, and one set of
governance controls (quotas, retention, RBAC, audit).

Built as a hexagonal/DDD Rust backend (`artiferris-domain` → `artiferris-application`
→ `artiferris-infrastructure`/`artiferris-api`, plus protocol adapter crates
`artiferris-npm` and `artiferris-docker`) with an Angular 22 frontend served from
the same binary.

## Table of contents

- [Features](#features)
- [Architecture](#architecture)
- [Tech stack](#tech-stack)
- [Running locally](#running-locally)
- [Deploying](#deploying)
- [Configuration reference](#configuration-reference)
- [Security](#security)
- [Comparison with alternatives](#comparison-with-alternatives)
- [Roadmap](#roadmap)
- [License](#license)

## Features

**Registries**
- npm registry protocol (publish, install, unpublish, dist-tags)
- Docker/OCI registry protocol (push, pull, manifest delete)
- Three repository types per format: **hosted** (you own the content),
  **proxy** (transparent cache in front of an upstream registry, e.g.
  npmjs.org or Docker Hub), **group** (aggregates several repositories
  behind one endpoint)
- Per-repository storage quotas (Docker counts what a repository holds: blobs
  its manifests reference, blobs uploaded but not yet referenced, manifest
  bodies, 1 KiB per tag, and the bytes of uploads in progress; at most 32
  uploads may be open per repository and 10,000 tags). An npm package holds at
  most 5,000 versions and 32 MiB of version manifests
- Per-repository retention policy (keep the last N versions/tags per
  package/image; tagged references such as `latest` are never purged) —
  swept automatically every 6 hours. Docker tags are ranked by when the tag
  itself was set, so moving a tag back to an old image (a rollback) makes it
  the newest. In a repository with a policy, manifests that no tag and no
  manifest list references are also removed 7 days after their last tag moved
  away (or after their push, if they never had one)
- Re-publishing an unpublished npm version is refused, whatever its bytes
- Package/image browser with per-package detail views: npm README (Markdown
  converted then sanitized on the server, links in a new tab, https-only
  images), install command, Docker tag sizes
- Public `artiferris-npm` and `artiferris-docker` catalogs (`/artiferris-npm`,
  `/artiferris-docker`), no authentication needed: search across every
  package/image of public hosted repositories, personal and organization
  alike. A catalog is not an install endpoint: each result points at its
  owner's own URL. Owner profile pages at `/@user` and `/o/<organization>`.
  The `artiferris-` prefix is reserved (repositories, users, organizations).
  Search-engine indexing (off by default)

**Security scanning**
- npm dependency audit against the public advisory database, auto-triggered
  on publish
- Docker image vulnerability scanning via [Trivy](https://github.com/aquasecurity/trivy),
  auto-triggered on push, plus manual rescan

**Multi-tenant**
- Organizations resolved by subdomain (`acme.artiferris.example` routes to the
  `acme` organization), each with its own repositories, users, and branding
- Organization admins scoped to their own organization only; a super-admin
  can target any organization via `?organization_id=`
- Public organization as the default for single-tenant deployments

**Auth & access control**
- Local accounts with Argon2 password hashing
- SSO: LDAP/Active Directory and OIDC (OpenID Connect), alongside local
  accounts — SAML isn't supported yet
- Mandatory second factor: TOTP or WebAuthn/passkeys, with one-time backup
  codes
- Personal API tokens (scoped per user; admins can list/revoke any token)
- Per-repository RBAC (`read` / `write` / `admin`), granted per user
- Email-based account invitations and activation flow
- Per-process login throttling on failed attempts

**Admin console**
- Usage metrics, hourly metrics-history snapshots, and a health-status page
- Audit log and security-event log
- User management (create, delete, promote to super-admin)
- SMTP settings (host/port/credentials/security/from-name/from-address)
  with a test-email button
- Custom branding: replace the default logo/favicon everywhere (UI +
  emails) for white-label / isolated-cluster deployments
- Configuration export/import for backup and restore, single-tenant instances
  only. The export refuses an instance whose users have personal repositories
  (the file has no owner for a repository). The file may be up to 32 MiB and
  list at most 100,000 users, repositories and permissions each; invitation
  emails go out after the commit, eight at a time. The import runs in one
  transaction: entries it cannot restore are listed and skipped, the rest lands
  all together or not at all, so a failed import can be run again
- Transactional HTML emails (account created, password reset, MFA/passkey
  enrolled) with the deployment's branding embedded inline (CID, so it
  renders even with external images blocked)

## Architecture

Hexagonal/DDD, dependencies point inward:

| Crate | Role |
|---|---|
| `artiferris-domain` | Entities, value objects, ports (traits) — no framework or I/O dependencies |
| `artiferris-application` | Use cases, orchestrating domain logic against ports |
| `artiferris-infrastructure` | Port implementations: Postgres, filesystem storage, SMTP, Trivy, Argon2, JWT |
| `artiferris-api` | Axum HTTP server, route handlers/DTOs, wires everything together, serves the built frontend |
| `artiferris-npm` | npm registry protocol adapter (its own route set, mounted into `artiferris-api`) |
| `artiferris-docker` | Docker/OCI registry protocol adapter (same pattern) |

Repositories (`PackageRepository`) and permissions are event-sourced;
most other state (users, settings, audit log, metrics) is plain CRUD over
Postgres.

## Tech stack

- **Backend:** Rust (edition 2024), [Axum](https://github.com/tokio-rs/axum), [SQLx](https://github.com/launchbadge/sqlx) + Postgres, [webauthn-rs](https://github.com/kanidm/webauthn-rs), [lettre](https://github.com/lettre/lettre)
- **Frontend:** Angular 22, standalone components, signals (no NgRx)
- **Storage:** local filesystem (`StorageBackendPort` is abstracted, but only a filesystem implementation exists today — see [Roadmap](#roadmap))

## Running locally

```bash
git clone <this-repo>
cd hangar
cp .env.example .env   # fill in POSTGRES_PASSWORD / JWT_SECRET
./scripts/dev.sh
```

This starts Postgres via `docker-compose`, runs the migrations, then runs
the backend (`cargo run -p artiferris-api`, port 8081) and the frontend
(`ng serve`, port 4200 with hot reload) side by side. `scripts/dev.sh` sets
`ARTIFERRIS_BOOTSTRAP_ADMIN_USERNAME=admin` / `ARTIFERRIS_BOOTSTRAP_ADMIN_PASSWORD=admin123`
for you, so you can sign in immediately.

Run tests:

```bash
cargo test --workspace                  # backend
npm test --prefix frontend              # frontend
```

## Deploying

### Docker Compose

A single-node deployment (ArtiFerris + Postgres) is one command away:

```bash
cp .env.example .env   # fill in POSTGRES_PASSWORD, JWT_SECRET, SECRETS_ENCRYPTION_KEY, PUBLIC_URL, ARTIFERRIS_BOOTSTRAP_ADMIN_*
docker compose up -d --build
```

The server refuses to start with a `JWT_SECRET` or `SECRETS_ENCRYPTION_KEY`
shorter than 32 bytes or still starting with `change-me`, with the same
placeholder as `ARTIFERRIS_BOOTSTRAP_ADMIN_PASSWORD`, or with a `PUBLIC_URL`
pointing at `0.0.0.0`. Generate the secrets with `openssl rand -base64 48`.
`POSTGRES_PASSWORD` ends up unescaped inside `DATABASE_URL`, so give it only
URL-safe characters (`openssl rand -hex 24`).

The container's health check calls `/readyz` (the database answers, without
waiting for a free connection: a fully busy pool still counts as up while a
recent check succeeded); `/healthz` only says the process is up.

See [Configuration reference](#configuration-reference) below for every
variable `docker-compose.yml` wires through. The Docker registry protocol's
token realm (`ARTIFERRIS_DOCKER_TOKEN_REALM`) is derived automatically per
request and normally needs no configuration at all — set it explicitly only
if this deployment sits behind something that doesn't forward the original
`Host` header and scheme faithfully.

### Kubernetes (Helm)

```bash
helm upgrade --install artiferris ./helm/artiferris \
  --namespace artiferris --create-namespace \
  --set image.tag=<release tag> \
  --set ingress.host=app.example.com --set ingress.wildcardHost='*.example.com' \
  --set artiferris.baseDomain=example.com
```

- `image.tag` has no default: pick a release. Set `image.digest` to pin the
  image by digest (CI does this). A `latest` tag is always pulled.
- The chart generates the database password, `JWT_SECRET` and
  `SECRETS_ENCRYPTION_KEY` once, in `<release>-secrets`, using Helm's
  `lookup` to find them again on upgrade. `lookup` returns nothing under
  `helm template`, `--dry-run`, Argo CD or Flux, so with those tools set
  `secrets.postgresPassword`, `secrets.jwtSecret` and
  `secrets.secretsEncryptionKey` yourself, or every render invents new ones
  and every stored secret becomes unreadable. The Secret is annotated
  `helm.sh/resource-policy: keep`, so `helm uninstall` leaves it in place.
  The two PersistentVolumeClaims (registry data and database) are kept the
  same way while `persistence.keepOnUninstall` is `true` (the default); delete
  them by hand to wipe the data, or set it to `false` to have `helm uninstall`
  delete them.
  A `secrets.secretsEncryptionKey` you pass replaces the stored one (that is
  how a rotation supplies the new key), so leave it out of later upgrades
  unless you mean to change it.
- Pods run as uid 100 / gid 101 (the ids the image pins), without
  privilege escalation or capabilities, with a read-only root filesystem
  and the `RuntimeDefault` seccomp profile. A NetworkPolicy lets only the
  ArtiFerris pod reach Postgres (`networkPolicy.enabled`; needs a CNI that
  enforces them).
- The ingress uses the Traefik middlewares listed in `ingress.middlewares`
  (`traefik-https`, `traefik-headers`, `traefik-ratelimit`, all in the
  `traefik` namespace by default). They must already exist; the chart does
  not create them. Set `ingress.middlewares` to an empty string to use none.
- Set `artiferris.trustedProxyIps` to the ingress controller's pod network,
  see `TRUSTED_PROXY_IPS`. With the ingress enabled the chart refuses to
  render while it is empty (every visitor would share one login throttle
  bucket and one public-catalog request budget); set
  `artiferris.allowSharedThrottleBucket=true` to accept that. The server also
  logs a warning, at most once an hour, when requests carry `X-Forwarded-For`
  from a private address while `TRUSTED_PROXY_IPS` is empty.
- A `startupProbe` gives a slow start (migrations, secret check) five minutes
  before the liveness probe takes over.
- The CI deploy runs `helm upgrade --install --atomic --cleanup-on-fail`, so a
  failed upgrade rolls the release back to the previous revision. Database
  migrations that already ran stay applied: after the first deploy of the
  release that introduced migrations 0006 to 0010, the previous image cannot
  start against the database, so the rolled-back release does not become
  ready either and the backup has to be restored (see "Upgrading and
  rotating" below).

## Configuration reference

Every variable `artiferris-api` reads from its environment.
`docker-compose.yml` already wires through the ones needed for a
single-node deployment; this table is the authoritative reference for a
bare-container deployment or for overriding those defaults.

| Env var | Required | Default | Description |
|---|---|---|---|
| `DATABASE_URL` | **Yes** | — | Postgres connection string, e.g. `postgres://user:pass@host:5432/artiferris`. |
| `JWT_SECRET` | **Yes** | — | Signs session tokens and Docker registry access tokens. At least 32 bytes of random data (`openssl rand -base64 48`); a shorter value or one starting with `change-me` stops the server at startup. Rotating it invalidates every session and every `docker login`. |
| `SECRETS_ENCRYPTION_KEY` | **Yes** | — | Encrypts what the database stores in recoverable form: SMTP passwords, LDAP and OIDC secrets, proxy-repository credentials and TOTP seeds. Different from `JWT_SECRET`, at least 32 bytes, not starting with `change-me`. It is expanded with HKDF, which does not make a weak value stronger: it must be random (`openssl rand -base64 48`), not a passphrase. See [Rotating `SECRETS_ENCRYPTION_KEY`](#rotating-secrets_encryption_key). |
| `SECRETS_ENCRYPTION_KEY_PREVIOUS` | No | — | Only while rotating: the key the stored secrets are currently encrypted with. |
| `SECRETS_REENCRYPT_LEGACY` | No | `false` | `true` lets the startup pass rewrite stored secrets to the current format and, after a rotation, under the new key. Off by default because the previous release cannot read the rewritten values: see [Rotating `SECRETS_ENCRYPTION_KEY`](#rotating-secrets_encryption_key). |
| `ARTIFERRIS_SSRF_ALLOWED_CIDRS` | No | — | Comma-separated addresses or CIDR ranges (`10.20.0.0/16,192.168.1.5`) that LDAP, SMTP, OIDC and proxy-repository hosts may resolve to even though they are private. Without it every private, loopback and link-local address is refused, which rules out an on-premises directory or mail relay. The OIDC issuer and every endpoint in its discovery document must be `https` and pass the same check. Unencrypted SMTP (`security: none`) is only accepted for hosts in this list. A typo stops the server at startup, and so does a `/0` range (it would switch the guard off). |
| `TRUSTED_PROXY_IPS` | No | — | Comma-separated addresses or CIDR ranges of the reverse proxies whose `X-Forwarded-For` is believed. A typo stops the server at startup, and so does a `/0` range (it would trust the whole internet). |
| `DB_MAX_CONNECTIONS` | No | `10` | Size of the Postgres connection pool; one connection is always kept open. A value that is not a positive number stops the server at startup. |
| `ARTIFERRIS_AUDIT_BACKFILL_FORCE` | No | `false` | The job that stamps the organization on older audit events does not start with `DB_MAX_CONNECTIONS` below 3 (it would compete with requests for the pool). `true` runs it anyway. |
| `ARTIFERRIS_BASE_DOMAIN` | **Yes** | — | Base domain organizations are resolved as subdomains of (e.g. `artiferris.example`, so `acme.artiferris.example` resolves the `acme` organization). No fallback: a misconfigured deployment must fail at startup rather than silently route every subdomain to the public organization. |
| `STORAGE_ROOT` | No | `./data` | Filesystem path where npm tarballs and Docker blobs are stored. Must be a persistent volume in any real deployment. |
| `BIND_ADDR` | No | `0.0.0.0:8080` | Address/port the HTTP server listens on. |
| `STATIC_DIR` | No | `./static` | Path to the built frontend assets served for non-API routes. Only relevant if you're not using the shipped Docker image. |
| `RUST_LOG` | No | — (no logging without it) | `tracing_subscriber` env-filter, e.g. `info` or `artiferris_api=debug,info`. Without it, the container logs almost nothing. |
| `CORS_ALLOWED_ORIGIN` | No | permissive (any origin) | Locks CORS to one origin. Leave unset for local dev (`ng serve` on a different port than the backend) or when the frontend is served from the same origin as the API (the shipped image's default setup). |
| `ARTIFERRIS_DOCKER_TOKEN_REALM` | No | derived per request from that request's own `Host` header and `PUBLIC_URL`'s scheme | Overrides the realm URL embedded in every `WWW-Authenticate` challenge, which the Docker CLI resolves `login`/`push`/`pull` token requests against — normally derived automatically so it's correct for however many organization subdomains this deployment serves. Set it only when the request's `Host`/scheme can't be trusted (e.g. an intermediary that doesn't forward them faithfully); doing so pins every client to this one fixed realm, which then breaks auth for every organization subdomain except whichever one this host happens to resolve to. Must be `https://` for any non-localhost host (`docker` refuses plain `http://` otherwise). |
| `PUBLIC_URL` | Yes, except in local development | derived from `BIND_ADDR` | The instance's external URL: base of invitation links, scheme of the Docker token realm and of HSTS, passkey origin, canonical URLs. The server refuses to start when it resolves to `0.0.0.0` on a base domain that is not `localhost` or `*.localhost`. |
| `ARTIFERRIS_BOOTSTRAP_ADMIN_USERNAME` | No | — | Username for the account auto-created **only when the `users` table is empty**. Safe to leave set across restarts/upgrades. |
| `ARTIFERRIS_BOOTSTRAP_ADMIN_PASSWORD` | No, but you need *some* way to get a first admin | — | Password for that same bootstrap account. Must be ≥ 8 characters — a shorter value fails silently (logged, not fatal) and leaves the deployment with no admin at all. A value starting with `change-me` (the `.env.example` placeholder) stops the server at startup. |

`PUBLIC_URL` falls back to guessing a URL from `BIND_ADDR`, which is only
ever correct for local development — set it explicitly in every other case.
It also supplies the scheme (`http`/`https`) `ARTIFERRIS_DOCKER_TOKEN_REALM`'s
automatic per-request derivation uses.

### Upgrading and rotating `SECRETS_ENCRYPTION_KEY`

Stored secrets (SMTP, LDAP, OIDC and proxy credentials, TOTP seeds) are
written in a versioned format that releases before this one cannot read. A
value saved after the upgrade already uses it, and reads accept both formats,
but existing values are only rewritten when `SECRETS_REENCRYPT_LEGACY=true`
(Helm: `artiferris.reencryptStoredSecrets`). Without it the server logs a
warning with the number of values still pending and changes nothing.

To upgrade:

1. Back up the database.
2. Deploy the new release with `SECRETS_REENCRYPT_LEGACY` unset.
3. Check that SSO, mail and MFA still work.
4. Set `SECRETS_REENCRYPT_LEGACY=true` and restart. The log line
   `re-encrypted stored secrets` gives the count. It can stay set: with
   nothing left to convert it does nothing.

**Rolling back the image after the first start of this release needs a
database restore.** The start applies migrations 0006 to 0010, and the previous
release (0.4.6 and earlier) refuses to start on a database that records
migrations it does not know ("failed to run migrations"), whether or not
`SECRETS_REENCRYPT_LEGACY` was ever set. `helm rollback`, `--atomic` and
redeploying the old tag do not undo migrations; back up before the upgrade and
restore that backup to go back. From this release on the server ignores
migrations recorded by a newer release, so rolling back to this one starts; it
then runs against the newer schema, which only works if that schema change was
additive (the release notes say when it is not).

The secret format is a second limit: releases before this one cannot read
values in the new format, so TOTP seeds (MFA logins fail), SMTP, LDAP, OIDC and
proxy credentials stop working. It applies to every value saved by this
release, and to the existing ones once `SECRETS_REENCRYPT_LEGACY` rewrites
them.

To change the key:

1. Back up the database.
2. Set `SECRETS_ENCRYPTION_KEY` to the new random value,
   `SECRETS_ENCRYPTION_KEY_PREVIOUS` to the value in use until now, and
   `SECRETS_REENCRYPT_LEGACY=true`. With Helm:
   `--set secrets.secretsEncryptionKey=<new> --set secrets.secretsEncryptionKeyPrevious=<old> --set artiferris.reencryptStoredSecrets=true`.
3. Restart every replica together. A replica still running the old key cannot
   read a value that another one has already moved to the new key, so a
   rolling restart with mixed keys is not safe.
4. The log line `re-encrypted stored secrets` confirms the move; an error
   line means some values could not be read with either key and were left
   untouched.
5. Remove `SECRETS_ENCRYPTION_KEY_PREVIOUS` (with Helm, drop
   `secretsEncryptionKeyPrevious` and upgrade again: the chart removes it
   from the Secret and keeps the new key).

A value that cannot be read with the keys the server has (a key that differs
from the one that sealed it, for instance after a restore) is logged at error
level with its type and the organization or user it belongs to, and the SSO
and SMTP settings endpoints report `secret_unreadable: true`. Whatever needs
it (SSO login, mail, TOTP checks, proxy credentials) stays broken until the
right key is provided or an admin enters the secret again.

**Not environment-configurable today** (hardcoded): the metrics-snapshot
sweep (hourly), the retention-policy sweep (every 6 hours) and the upload
sweep (hourly), which removes expired upload sessions, plus Docker blobs
that no manifest referenced within 48 hours of their upload. The Docker
image scanner shells out to a `trivy` binary that must be on `PATH` (the
shipped Dockerfile installs it; a custom image build needs to install it
too).

## Security

- Argon2 password hashing
- Mandatory MFA (TOTP or WebAuthn/passkeys) with one-time backup codes
- A distinct, short-lived "password verified but not 2FA verified" token
  type, kept deliberately non-interchangeable with a full session token
- Per-repository RBAC, enforced on every route — not just hidden in the UI
- Audit log and security-event log for admin review
- Request bodies (Docker uploads and manifests, npm publish, dist-tag and
  unpublish documents) are read only after the repository, role and scope
  checks pass. Docker blobs are streamed to disk; documents held in memory
  draw on a shared 1 GiB budget that is charged as the body arrives, and a
  request that does not fit gets a `503`; a client or user with four bodies
  already in flight gets a `429`
- Docker access tokens live 2 minutes (`expires_in` says so); a token that is
  presented but expired, forged or revoked gets a `401` challenge, so the
  client fetches a new one. Reading through a Docker group requires a `Read`
  grant on each member, checked live, like npm
- Blobs and manifests by digest of **public** repositories are sent
  `Cache-Control: public, max-age=31536000, immutable` (the content behind a
  digest never changes); npm tarballs of public repositories get
  `public, max-age=86400` only, because a cache that keeps a copy outlives the
  repository turning private, and a day is the compromise. A private
  repository's content is `private, no-store` and `Vary: Authorization`. A
  blob cached by a CDN can therefore stay there after a repository turns
  private: put the registry behind a cache that honours purges, or do not
  cache `/v2/` at the edge, if that matters
- Every `/api` response is `Cache-Control: no-store` (unless the handler chose
  its own, as the public catalog and the branding images do) and
  `Vary: Authorization`, since what it says depends on the caller. Every
  response also carries `Permissions-Policy` (camera, microphone, geolocation,
  payment, USB and other sensors denied) and
  `Cross-Origin-Opener-Policy: same-origin` (sign-in is a full-page redirect,
  nothing uses `window.opener`); the app, the static files and the JSON API
  add `Cross-Origin-Resource-Policy: same-origin`, but the npm and Docker
  registries and the branding images do not (other clients, proxies and link
  previews fetch them)
- The JSON API drops a request whose body stalls for 30 seconds or takes more
  than 120 seconds in total (`408`); the npm and Docker uploads have their own
  streaming timeouts. Connection counts and body timeouts at the edge remain
  the job of the ingress or reverse proxy
- `npm audit` from the UI is limited to 30 runs per minute per signed-in
  account, waits at most 10 seconds for one of its 4 outbound slots (`503`
  after that), and makes one call to npm per package at a time. Its results
  are cached for 10 minutes, with a separate partition for public
  repositories, the only ones anonymous visitors can read
- Proxy repositories send their stored credentials only to the configured
  remote itself (same scheme, host and port, over `https`); a tarball or a
  token realm on another host is fetched anonymously (so a Docker Hub proxy
  with credentials pulls public images anonymously), and a remote that would
  carry credentials over plain `http` is refused
- Download counters count one client (an IPv4 address, an IPv6 /64) once per
  package or image per hour, held in memory and never stored. Behind a
  reverse proxy, set `TRUSTED_PROXY_IPS` so the client is the forwarded
  address
- The audit log keeps security and administrative events for 365 days
  (`AUDIT_RETENTION_DAYS`, `0` keeps everything, at most 36500); a sweep
  five minutes after startup and then daily deletes older ones. Package,
  repository and permission events are never deleted. After an upgrade from a
  release without per-organization audit scoping, a background job stamps the
  organization on the older events; until it has finished (logged at startup),
  an organization admin's audit view does not list them yet. One replica works
  at a time (a lease it renews after every batch; another takes over five
  minutes after it stops), it holds a pool connection only while a batch runs,
  and it does not start with `DB_MAX_CONNECTIONS` below 3 unless
  `ARTIFERRIS_AUDIT_BACKFILL_FORCE=true`. Every stamped row is rewritten, so on a
  large history the event table and its indexes grow by roughly half until
  autovacuum reclaims the space (measured: 2 million events, about 13 minutes) Failed logins are
  recorded with the name that was typed (cut to 64 characters), so a password
  pasted into the username field can end up there, readable by super-admins for
  the retention period. They belong to no organization (only super-admins see
  them), and an access denial is filed under the organization of the person
  denied, not of the repository they tried
- The npm dependency scan and `npm audit` relay send package names and
  versions to `registry.npmjs.org`; packages this repository publishes itself
  are never looked up by the scan, and the relay is capped in size. Do not
  point `npm audit` at ArtiFerris if those names must not leave your network
- Format-sniffed (magic-byte) validation on uploaded assets (branding
  logo/favicon), never trusting a client-supplied `Content-Type`

Package READMEs are sanitized server-side, but their images may be loaded from
any `https` host (`img-src 'self' data: blob: https:` in the CSP). On public
pages every visitor therefore reveals their IP address and User-Agent to
whichever image host the package author picked. This is an accepted risk; to
tighten it, restrict `img-src` in `crates/artiferris-api/src/main.rs` (for
example to `'self' data: blob:`, which blocks remote images).

Found a security issue? Please report it privately rather than opening a
public issue.

## API Reference — Authentication

- **`POST /api/auth/login`** — authenticates a user with credentials (username + password); returns an MFA-enrollment response on success.
- **`POST /api/auth/register`** — self-registers a new account in the public organization (disabled on any other organization's subdomain); returns the same MFA-enrollment response as login.

## Search-engine optimisation (SEO)

The public catalog pages (`/explorer`, `/artiferris-npm`, `/artiferris-docker`, `/@user`, `/o/organization` and their repositories, packages and images) are served with their own `<head>`: title, description, canonical URL, Open Graph, Twitter card and schema.org structured data for packages. Page bodies are still rendered in the browser.

**Indexing is off by default.** A super-admin turns it on in Administration, settings of the public organization ("Référencement"). While off, every page sends `noindex`, `robots.txt` disallows everything and the sitemap is empty. Once on, `/robots.txt` disallows the API, the registries and the app, and `/sitemap.xml` (an index of files of at most 40,000 URLs, cached for 10 minutes) lists the public owners, repositories, packages and images, 500,000 of them at most. Search-result pages (`?q=`) stay `noindex`.

The sitemap is rebuilt by one request at a time; the others get the previous copy meanwhile, which is also kept if a rebuild fails. The indexing setting is read at most every 30 seconds and the last known value is used when the database cannot answer; if there never was one, `robots.txt` and the sitemap answer `503` rather than pretending the catalog is closed. A page head that takes more than 300 ms to build is replaced by the generic one, so the app always loads.

Canonical and sitemap URLs are built from `PUBLIC_URL`, which must be the instance's real public address. A private, unknown or deleted repository gets the same generic `<head>` as any app page, so nothing is revealed.

**The page's language** follows the request's `Accept-Language` header (`fr-CA` gives French; the first translated language in order of preference wins: en, fr, es, it, de) and is English when there is no header or no translated language. That is the case of a crawler, which usually sends none: English is therefore the language a search engine indexes. The URL is unique: the canonical tag is the same in every language, the response carries `Vary: Accept-Language`, and `<html lang>`, `og:locale` and `inLanguage` (structured data) follow the chosen language. A generic page (app, private or unknown repository) says nothing in any language and keeps the document's `lang`.

**Blocking robots and the public page, per organization and for the instance.** In Administration, System settings of an organization, an organization admin can close its public page ("Public page"): its catalog pages then answer as if nothing was published (not found for visitors, absent from search, suggestions, counts and the sitemap). They can also keep search engines away ("Keep search engines away from this organization"): its pages stay visible, send `noindex, nofollow` and are left out of the sitemap. Both are open by default. On the public organization, "Public page" closes the catalog of the whole instance (super-admins only): no page is served, `robots.txt` disallows everything and the sitemap is empty, as when indexing is off. The instance's indexing switch stays the master: an organization cannot index itself if the instance does not allow it. Personal repositories (`/@user`) follow the instance's settings.

## API Reference — Package and image details

Downloads are counted as they happen: a `GET` of an npm tarball, or a `GET` of a Docker manifest by tag, served straight from a hosted repository (not `HEAD`, not a request by digest, not a proxy or group repository). A client counts once per package or image per hour. Counters are aggregated in memory and written every 30 seconds and on shutdown, per day, with no data about the user; figures are indicative. Days older than 13 months are pruned.

- **`GET /api/repositories/{id}/packages/npm/{name}`** — versions, dist-tags, `downloads_7d` (downloads over the last 7 days), `readme_html` (the latest version's README, converted and sanitized on the server; `null` when there is none. Only the first 128 KB are read, and a README that nests too deeply or holds too much markup is shown as plain escaped text instead) and `registry_url` (the owner's registry, to use with `npm install`).
- **`GET /api/repositories/{id}/packages/docker/{image}`** — `downloads_7d`, tags (with `size_bytes`: config plus layers, `null` for a multi-architecture index) and `image_reference` (the reference to pull, without a tag).

Both are limited to 60 requests per minute per IP for anonymous callers (429 beyond that); signed-in callers are not limited here.

## API Reference — Public catalog

No authentication, `Cache-Control: no-store`, limited to 60 requests per minute per IP (429 with `Retry-After` beyond that). The landing lists (a search without `q`) and the entry counts are kept in memory for 30 seconds, so a repository made private can stay listed for that long; its content is still protected by the access checks.

- **`GET /api/public/catalogs`** — one catalog per supported format (`format`, `name`, `label`, `entry_count`).
- **`GET /api/public/search`** — searches the npm packages and Docker images of public hosted repositories. Parameters: `q` (name, description, keywords or tags; 100 characters max), `format` (`npm` or `docker`), `owner` (`personal:<username>` or `organization:<slug>`), `sort` (`relevance`, `updated` or `popular`), `page` (1 to 100), `per_page` (1 to 50, default 20). Search tolerates typos in names (from 3 characters; `match_kind` is then `fuzzy` and those results come last). Each result also carries `downloads_7d`, the downloads over the last 7 days. Each result names its owner, its source repository and its install location (`registry_url` for npm, `image_reference` for Docker).
- **`GET /api/search`** — the same search for a signed-in user, over everything they can read: their organization's repositories (proxies included: a proxy only contributes what it has already cached), public repositories and those they hold a grant on. Same parameters, ranking and response shape; each `repository` also carries its `id` and `repo_type` (`hosted` or `proxy`). A super-admin searches everywhere. Needs a token (401 otherwise).
- **`GET /api/public/suggest`** — name suggestions while typing (`q`: 2 to 100 characters). At most 8 results (`kind`, `name`, repository and owner), best first: exact name, prefix, substring, then approximate. Own budget: 120 requests per minute per IP.
- **`GET /api/public/owners/{kind}/{slug}`** — public summary of an owner (`kind`: `personal` or `organization`): display name and counts of repositories, packages and images. 404 when they have no public repository, which is also the answer for an unknown owner, so the page can't confirm that an account exists.
- **`GET /api/repositories/by-org/{slug}/{repo_name}`** — a public repository of an organization (404 otherwise), the counterpart of `by-owner` for personal projects. Like `by-owner` and `GET /api/repositories/{id}`, an anonymous caller gets the repository without `quota_bytes`, `retention_keep_last_n` and `organization_id`; a signed-in caller keeps the full shape.

## Comparison with alternatives

ArtiFerris isn't the only option for self-hosting an npm and/or Docker
registry. Here's where it stands against three industry references — on
feature scope and license cost, not performance numbers: no comparative
benchmark has been run across these four tools, and it would be
dishonest to make one up.

| | **ArtiFerris** | Nexus Repository (Community Edition) | Harbor | JFrog Artifactory |
|---|---|---|---|---|
| npm | ✅ | ✅ | ❌ | Paid (Pro) only |
| Docker / OCI | ✅ | ✅ | ✅ | Paid (Pro) only |
| Other formats (Maven, PyPI, NuGet, Cargo, Helm…) | ❌ *(roadmap)* | ✅ 20+ formats | OCI only (Helm, SBOM, OPA…) | ✅ 60+ formats *(Pro)* |
| MFA | **Mandatory**, built-in (TOTP/passkey) | Optional, SSO in Pro | Optional | Optional, SSO in Pro |
| LDAP/OIDC | ✅ built-in | Pro | ❌ | Pro |
| SAML | ❌ *(roadmap)* | Pro | ❌ | Pro |
| Built-in vulnerability scanning | ✅ Trivy, built-in | Separate product (Sonatype Lifecycle) | ✅ Trivy, built-in | Paid (Xray) |
| Multi-tenant / isolated projects | ✅ subdomain-based organizations | ✅ | ✅ | ✅ |
| Free self-hosting | ✅ | ✅ (Community Edition) | ✅ (Apache 2.0, CNCF project) | Java only — Docker/npm require the paid tier |

**Estimated annual cost, self-hosted, excluding infrastructure and
operations** (sourced figures — most of these vendors have no public
list price, see the notes):

- **ArtiFerris** — free, no license.
- **Harbor** — free, Apache 2.0, CNCF project, no paid tier at all.
- **Nexus Repository Community Edition** — free for npm, Docker, Maven,
  PyPI, and about fifteen other formats. Pro (SSO, high availability,
  replication) has no public price; third-party estimates put it around
  $120/user/year, or $50,000–$150,000+/year bundled with the full
  Sonatype platform[^nexus-pricing].
- **JFrog Artifactory** — the open-source edition (Apache 2.0) only
  covers the Java ecosystem (Maven/Gradle/Ivy): no Docker, no npm. To
  get the two formats ArtiFerris covers natively and for free, you need
  Pro X, whose published self-hosted starting price is $27,000/year for
  one server[^jfrog-pricing], climbing well beyond that at enterprise
  scale.

[^nexus-pricing]: [Sonatype Nexus Repository Pricing Guide — CloudRepo](https://www.cloudrepo.io/articles/sonatype-nexus-repository-pricing-guide)
[^jfrog-pricing]: [JFrog Artifactory Pricing Guide — CloudRepo](https://www.cloudrepo.io/articles/jfrog-artifactory-pricing-guide)

**Recommended hardware configuration** (figures taken from each
product's official documentation, not from a comparative benchmark):

| | **ArtiFerris**[^artiferris-bench] | Nexus Repository (Community Edition) | Harbor | JFrog Artifactory (Pro X, self-hosted) |
|---|---|---|---|---|
| Minimum CPU | 0.5 core | 2 cores ("Small" profile)[^nexus-sysreq] | 2 cores[^harbor-prereqs] | 4 cores, up to 20 active clients[^jfrog-sizing] |
| Recommended CPU | 1 core | 4 to 8 cores depending on profile[^nexus-sysreq] | 4 cores[^harbor-prereqs] | 6 to 8 cores, up to 200 active clients[^jfrog-sizing] |
| Minimum RAM | 128 MB | 8 GB[^nexus-sysreq] | 4 GB[^harbor-prereqs] | 6 GB, up to 20 active clients[^jfrog-sizing] |
| Recommended RAM | 256 MB | 8 to 32 GB depending on profile[^nexus-sysreq] | 8 GB[^harbor-prereqs] | 12 to 18 GB, up to 200 active clients[^jfrog-sizing] |
| Disk | not measured | ≥ 4 GB free at all times (falls back to read-only otherwise); 500 GB+ common with Docker/Maven[^nexus-sysreq] | 40 GB minimum, 160 GB recommended[^harbor-prereqs] | not quantified in the general docs; SSD recommended[^jfrog-sysreq] |
| Database | PostgreSQL, required | Embedded H2 for evaluation, PostgreSQL recommended in production[^nexus-sysreq] | PostgreSQL bundled with the installer | External PostgreSQL, required in production[^jfrog-sysreq] |
| Runtime | Native Rust binary, no JVM | JVM, Java 21 required[^nexus-sysreq] | Go, several containers, no JVM | JVM, bundled JDK 21[^jfrog-sysreq] |

[^artiferris-bench]: Measured, not documented: the `artiferris-api` container
    capped via `docker run --cpus`/`--memory` (cgroup v2), against 15-20
    simulated clients (npm install/publish + docker pull/push, mostly
    reads) for 2-3 minutes. RAM and CPU read directly from the
    container's `/sys/fs/cgroup/memory.current` and `cpu.stat`, not
    estimated. "Minimum" = 0.5 core / 128 MB: the load completes with no
    application-level failure, but with noticeable CPU throttling
    (~68% of run time throttled) and RAM right at the ceiling.
    "Recommended" = 1 core / 256 MB: 2185 requests, 2 failures, residual
    throttling (~3% of run time), RAM with headroom (peaked at 92 MB).
    Measured on a development machine, not dedicated server hardware —
    not directly comparable to the other three vendors' methodology,
    which document sizing profiles for production deployments on
    dedicated hardware.
[^nexus-sysreq]: [Sonatype Nexus Repository System Requirements](https://help.sonatype.com/en/sonatype-nexus-repository-system-requirements.html)
[^harbor-prereqs]: [Harbor Installation Prerequisites](https://goharbor.io/docs/2.13.0/install-config/installation-prereqs/)
[^jfrog-sizing]: [JFrog Hardware Sizing Matrix](https://docs.jfrog.com/installation/docs/hardware-sizing-matrix)
[^jfrog-sysreq]: [JFrog General System Requirements](https://docs.jfrog.com/installation/docs/general-system-requirements)

### Scaling up

Still measured, not documented: the table above comes from a modest load
(15-20 clients). To see how ArtiFerris handles more concurrency, same
container (4 cores / 2 GB), but this time driven by an async HTTP load
generator (Python/aiohttp) instead of real npm/docker CLI processes per
client — that's what let this go up to 100 and 200 simultaneous clients
without multiplying heavy processes on the test machine[^artiferris-scale]:

| Concurrent clients | Throughput | Failures | p95 (npm install) | Avg CPU | RAM (peak) |
|---|---|---|---|---|---|
| 20 | ~490 req/s | 0 | 90 ms | 108% (of 4 cores) | 549 MB |
| 100 | ~476 req/s | 0 | 360 ms | 108% | 568 MB |
| 200 | ~268 req/s | 0 | 1,781 ms | 81% | 527 MB |

Zero application-level failures at every tier, including at 200 clients:
ArtiFerris slows down under heavy load but doesn't break. The less flattering
part, stated plainly: throughput **drops** between 100 and 200 clients
(476 → 268 req/s) while CPU usage drops too (108% → 81%) — a sign of a
bottleneck that isn't raw core count (likely the PostgreSQL connection
pool or contention on the async event loop, though not investigated
further). One run per tier, on a development machine: take it as an
order of magnitude, not a capacity guarantee.

**More CPU, more throughput** — still at 100 clients, doubling the CPU
allocation clearly moves sustained throughput:

| Config | Throughput at 100 clients | CPU throttling |
|---|---|---|
| 4 cores / 2 GB | ~476 req/s | noticeable |
| 8 cores / 2 GB | ~690 req/s | light |

No "recommended RAM for 100 clients" row here, deliberately: with a
client that fires with no rate limit at all, observed RAM grows with the
**test's duration** (an accumulating backlog of pending requests), not
with a stable per-client cost — over 15s it plateaued at 1.2 GB, over
60s it filled the 4 GB allocated. A trustworthy RAM figure would need a
load generator with a capped request rate (realistic requests/second
instead of full-throttle), which wasn't done here.

[^artiferris-scale]: Load generator: Python `aiohttp`, direct HTTP calls
    against the same endpoints a real client hits (npm metadata + tarball,
    Docker token + manifest + blob), bypassing the `npm`/`docker` CLIs
    entirely. Same mix as the note above (mostly reads). CPU/RAM read
    from the same cgroup counters as the table above.

**What these two tables don't say**: no comparative benchmark has been
run against Nexus, Harbor, or Artifactory — the figures above are ArtiFerris
only. Nexus and Harbor are also mature projects, deployed at scale for
years, with features ArtiFerris doesn't have yet (see the
[roadmap](#roadmap)): SAML, high availability, more package formats.

## Roadmap

ArtiFerris is an active project, not a finished product: quotas, retention,
built-in security scanning (Trivy + npm audit), customizable branding, and
mandatory MFA are already native. The list below is what we genuinely want
to build next — in rough priority order.

**Strengthening the foundations**
- [ ] SAML — LDAP/Active Directory and OIDC are already supported, SAML
      isn't yet
- [ ] More package formats: Maven/Gradle, PyPI, NuGet, Cargo, Go modules,
      Helm charts, generic/raw repositories — `artiferris-npm`/`artiferris-docker`
      already show the adapter pattern to follow
- [ ] Object-storage backend (S3-compatible) behind `StorageBackendPort`,
      to unblock multi-replica deployments (today: one filesystem volume,
      one replica)
- [ ] High availability / clustering, geo-replication
- [ ] Package signing / provenance (Sigstore, npm provenance)

**Thinking bigger: an open registry**
- [ ] Per-user namespaces within an organization (npm-style `@user/...`
      scopes), distinct from today's model where repositories belong to
      the organization
- [ ] Public, unauthenticated read access for public packages
- [ ] Rate limiting and abuse prevention for anonymous traffic
- [x] Public search and package-discovery pages
- [ ] CDN-backed global artifact distribution

## License

No license file is currently included in this repository — treat the
source as all-rights-reserved until one is added.
