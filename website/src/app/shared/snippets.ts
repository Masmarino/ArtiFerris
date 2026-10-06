// Code shown on the site. Same in every language: commands and configuration aren't translated.

/**
 * The client side of the group demo: the `.npmrc` lines a repository's Utilisation page gives
 * (docs/utilisation/npm.md), pointed at a group, and the install that goes through it.
 */
export const NPMRC_EXAMPLE = `registry=https://acme.artiferris.pro/npm/npm-all/
//acme.artiferris.pro/npm/npm-all/:_authToken=<jeton>

# then, in the project
npm install left-pad
`

/** From docs/utilisation/docker.md: log in with an API token, push, and pull. */
export const DOCKER_EXAMPLE = `echo <jeton> | docker login acme.artiferris.pro -u camille --password-stdin
docker tag api:1.4.0 acme.artiferris.pro/images/api:1.4.0
docker push acme.artiferris.pro/images/api:1.4.0
docker pull acme.artiferris.pro/images/api:1.4.0
`

/** The hero's one-liner: the command that starts the stack (docs/administration/installation.md). */
export const HERO_COMMAND = 'docker compose up -d --build'

/**
 * What the hero prints: the production instance measured on its cluster, copied as printed on 6 October 2026, version
 * 0.6.1, a few minutes after the server pod restarted.
 */
export const PROOF_TERMINAL = `$ kubectl top pod -n artiferris --containers
POD                                    NAME             CPU(cores)   MEMORY(bytes)
artiferris-7f497fb86-pcft9             artiferris-api   2m           5Mi
artiferris-postgres-7f7784f89f-rcd4s   postgres         9m           48Mi`

/** From docs/administration/installation.md. */
export const COMPOSE_COMMANDS = `git clone https://github.com/Masmarino/ArtiFerris.git
cd ArtiFerris
cp .env.example .env
# set POSTGRES_PASSWORD, JWT_SECRET, SECRETS_ENCRYPTION_KEY, PUBLIC_URL and the first admin in .env
docker compose up -d --build
`

/** From docs/administration/installation.md. */
export const HELM_COMMANDS = `git clone https://github.com/Masmarino/ArtiFerris.git
cd ArtiFerris
helm upgrade --install artiferris ./helm/artiferris \\
  --namespace artiferris --create-namespace \\
  --set image.tag=0.6.1 \\
  --set ingress.host=app.example.com --set ingress.wildcardHost='*.example.com' \\
  --set artiferris.baseDomain=example.com
`

/** From the "Lancer en local" section of the README: Postgres with Compose, the backend and the web application. */
export const SOURCE_COMMANDS = `git clone https://github.com/Masmarino/ArtiFerris.git
cd ArtiFerris
cp .env.example .env          # then set POSTGRES_PASSWORD and JWT_SECRET
./scripts/dev.sh              # PostgreSQL via Compose, the backend on :8081, ng serve on :4200
`

/**
 * The response of the production instance (version 0.6.1, behind Traefik) to the command, copied as printed on 6 October
 * 2026. The server sets the content policy, X-Frame-Options, X-Content-Type-Options, Referrer-Policy, Permissions-Policy
 * and the Cross-Origin policies (crates/artiferris-api/src/main.rs); Strict-Transport-Security and X-XSS-Protection come
 * from the proxy here.
 */
export const SECURITY_HEADERS = `$ curl -s -D - -o /dev/null https://app.artiferris.pro/healthz
HTTP/2 200
content-security-policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; font-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'
content-type: text/plain; charset=utf-8
cross-origin-opener-policy: same-origin
cross-origin-resource-policy: same-origin
date: Tue, 06 Oct 2026 14:31:01 GMT
permissions-policy: accelerometer=(), autoplay=(), camera=(), display-capture=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), midi=(), payment=(), usb=(), xr-spatial-tracking=()
referrer-policy: same-origin
strict-transport-security: max-age=31536000; includeSubDomains; preload
vary: origin
x-content-type-options: nosniff
x-frame-options: DENY
x-xss-protection: 1; mode=block
content-length: 2`
