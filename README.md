*[Read this in English](README.en.md)*

# ArtiFerris

Gestionnaire d'artefacts auto-hébergé : un registre npm et un registre Docker/OCI derrière une seule console
d'administration, une seule authentification et les mêmes contrôles (quotas, rétention, droits, audit).

Backend Rust en architecture hexagonale, frontend Angular 22 servi par le même binaire.

## Sommaire

- [Fonctionnalités](#fonctionnalités)
- [Architecture](#architecture)
- [Lancer en local](#lancer-en-local)
- [Déploiement](#déploiement)
- [Configuration](#configuration)
- [Mises à jour et rotation de la clé de chiffrement](#mises-à-jour-et-rotation-de-la-clé-de-chiffrement)
- [Sécurité](#sécurité)
- [Référencement (SEO)](#référencement-seo)
- [API publique](#api-publique)
- [Feuille de route](#feuille-de-route)
- [Site web](#site-web)
- [Licence](#licence)

## Fonctionnalités

**Registres**
- npm (publish, install, unpublish, dist-tags) et Docker/OCI (push, pull, suppression de manifest).
- Trois types de dépôt par format : **hosted** (votre contenu), **proxy** (cache devant un registre amont) et
  **group** (plusieurs dépôts derrière un seul point d'entrée).
- Quota de stockage et rétention par dépôt (les N dernières versions ou tags ; `latest` n'est jamais purgé ;
  purge toutes les 6 heures). Un paquet npm a au plus 5 000 versions et 32 Mio de manifestes.
- Republier une version npm dépubliée est refusé.
- Navigateur de paquets et d'images : README npm (converti et nettoyé côté serveur), commande d'installation,
  taille des tags Docker.
- Catalogues publics sans authentification (`/artiferris-npm`, `/artiferris-docker`), profils `/@utilisateur` et
  `/o/<organisation>`. Le préfixe `artiferris-` est réservé.

**Sécurité des artefacts**
- Audit des dépendances npm à chaque publication.
- Scan des images Docker avec [Trivy](https://github.com/aquasecurity/trivy) à chaque push, relançable à la main.

**Multi-tenant**
- Une organisation par sous-domaine (`acme.artiferris.example`), avec ses dépôts, ses utilisateurs et sa marque.
- Les admins d'organisation n'agissent que sur la leur ; un super-admin cible n'importe laquelle avec
  `?organization_id=`.
- Une organisation publique par défaut pour les déploiements mono-tenant.

**Comptes et accès**
- Comptes locaux (Argon2), LDAP/Active Directory et OIDC. SAML n'est pas supporté.
- Double authentification obligatoire (TOTP ou passkey) avec codes de secours.
- Tokens API personnels ; les admins peuvent lister et révoquer ceux de tous.
- Droits par dépôt (`read`, `write`, `admin`), invitations par e-mail (l'administrateur ne saisit que l'adresse, l'invité choisit son nom d'utilisateur à l'activation), limitation des connexions échouées.

**Administration**
- Métriques d'usage et historique, état de santé, journal d'audit et journal de sécurité.
- Utilisateurs, SMTP (avec e-mail de test), marque personnalisée (logo, favicon).
- Export et import de configuration (instances mono-organisation, sans dépôts personnels). L'import est une seule
  transaction : ce qu'il ne peut pas restaurer est listé et ignoré, le reste est écrit d'un bloc.
- E-mails HTML dans la langue du destinataire (compte créé, mot de passe régénéré, MFA ajouté).

**Langues** : interface en français, anglais, espagnol, italien et allemand. La langue du compte prime sur celle du
navigateur ; les e-mails et les titres des pages publiques suivent la langue du lecteur.

## Architecture

Les dépendances pointent vers l'intérieur.

| Crate | Rôle |
|---|---|
| `artiferris-domain` | Entités, objets de valeur et ports, sans framework ni I/O |
| `artiferris-application` | Cas d'usage |
| `artiferris-infrastructure` | Postgres, fichiers, SMTP, Trivy, Argon2, JWT |
| `artiferris-api` | Serveur Axum, routes, assemblage, sert le frontend |
| `artiferris-npm` | Protocole du registre npm |
| `artiferris-docker` | Protocole du registre Docker/OCI |

Les dépôts et les permissions sont en event-sourcing ; le reste (utilisateurs, paramètres, audit, métriques) est du
CRUD sur Postgres. Le stockage est un système de fichiers local (`StorageBackendPort` est abstrait, mais il n'existe
pas d'autre implémentation).

Stack : Rust (édition 2024), Axum, SQLx, Postgres, webauthn-rs, lettre ; Angular 22 (composants standalone, signals).

## Lancer en local

```bash
cp .env.example .env   # renseigner POSTGRES_PASSWORD et JWT_SECRET
./scripts/dev.sh
```

Le script démarre Postgres (`docker-compose`), applique les migrations, puis lance le backend (port 8081) et le
frontend (`ng serve`, port 4200). Un admin `admin` / `admin123` est créé.

```bash
cargo test --workspace        # backend
npm test --prefix frontend    # frontend
```

## Déploiement

### Docker Compose

```bash
cp .env.example .env   # POSTGRES_PASSWORD, JWT_SECRET, SECRETS_ENCRYPTION_KEY, PUBLIC_URL, ARTIFERRIS_BOOTSTRAP_ADMIN_*
docker compose up -d --build
```

Le serveur refuse de démarrer si `JWT_SECRET` ou `SECRETS_ENCRYPTION_KEY` fait moins de 32 octets ou commence par
`change-me`, si le mot de passe de l'admin de départ est encore le placeholder, ou si `PUBLIC_URL` pointe sur
`0.0.0.0`. Générez les secrets avec `openssl rand -base64 48` (`openssl rand -hex 24` pour `POSTGRES_PASSWORD`, qui
finit dans une URL).

`/readyz` vérifie que la base répond ; `/healthz` dit seulement que le processus tourne.

Mesuré sur une machine de développement avec 15 à 20 clients : 1 cœur et 256 Mo suffisent ; 0,5 cœur et 128 Mo
passent, avec un CPU bridé.

### Kubernetes (Helm)

```bash
helm upgrade --install artiferris ./helm/artiferris \
  --namespace artiferris --create-namespace \
  --set image.tag=<release> \
  --set ingress.host=app.example.com --set ingress.wildcardHost='*.example.com' \
  --set artiferris.baseDomain=example.com
```

- `image.tag` n'a pas de défaut ; `image.digest` épingle l'image (la CI le fait).
- Le chart génère le mot de passe de la base, `JWT_SECRET` et `SECRETS_ENCRYPTION_KEY` dans `<release>-secrets` et les
  retrouve avec `lookup`. Avec `helm template`, `--dry-run`, Argo CD ou Flux, `lookup` ne renvoie rien : fixez
  `secrets.postgresPassword`, `secrets.jwtSecret` et `secrets.secretsEncryptionKey`, ou chaque rendu en invente de
  nouveaux et les secrets stockés deviennent illisibles.
- Le Secret et les deux volumes (registre et base) survivent à `helm uninstall` (`persistence.keepOnUninstall`).
- Pods en uid 100 / gid 101, sans élévation de privilèges, système de fichiers en lecture seule, seccomp
  `RuntimeDefault` ; une NetworkPolicy ne laisse joindre Postgres qu'à ArtiFerris.
- L'ingress utilise les middlewares Traefik de `ingress.middlewares`, qui doivent exister ; une valeur vide n'en
  utilise aucun.
- Avec l'ingress, renseignez `artiferris.trustedProxyIps` (réseau de pods du contrôleur) : le chart refuse de se
  rendre sinon, car tous les visiteurs partageraient un seul compteur de connexions
  (`artiferris.allowSharedThrottleBucket=true` l'accepte).
- La CI déploie avec `--atomic --cleanup-on-fail`. Les migrations déjà appliquées restent : voir la section suivante
  pour revenir en arrière.

## Configuration

Variables lues par `artiferris-api`. `docker-compose.yml` câble celles d'un déploiement mono-nœud.

| Variable | Obligatoire | Défaut | Description |
|---|---|---|---|
| `DATABASE_URL` | Oui | | Connexion Postgres. |
| `JWT_SECRET` | Oui | | Signe les sessions et les tokens Docker. 32 octets aléatoires au moins. Le changer invalide toutes les sessions et tous les `docker login`. |
| `SECRETS_ENCRYPTION_KEY` | Oui | | Chiffre les secrets stockés (SMTP, LDAP, OIDC, identifiants proxy, graines TOTP). Différente de `JWT_SECRET`, aléatoire (HKDF ne renforce pas une phrase de passe). |
| `SECRETS_ENCRYPTION_KEY_PREVIOUS` | Non | | Pendant une rotation : la clé actuelle des secrets stockés. |
| `SECRETS_REENCRYPT_LEGACY` | Non | `false` | Autorise la réécriture des secrets stockés au format courant et sous la nouvelle clé. |
| `ARTIFERRIS_BASE_DOMAIN` | Oui | | Domaine de base des organisations (`artiferris.example` pour `acme.artiferris.example`). Sans valeur par défaut : une erreur de config doit échouer au démarrage, pas router tous les sous-domaines vers l'organisation publique. |
| `PUBLIC_URL` | Sauf en local | dérivée de `BIND_ADDR` | URL externe : liens d'invitation, schéma de HSTS et du realm Docker, origine des passkeys, URL canoniques. |
| `ARTIFERRIS_BOOTSTRAP_ADMIN_USERNAME` / `_PASSWORD` | Il faut un premier admin | | Compte créé seulement si la table `users` est vide. Mot de passe de 8 caractères au moins, pas `change-me…`. |
| `ARTIFERRIS_SSRF_ALLOWED_CIDRS` | Non | | Plages (`10.20.0.0/16,192.168.1.5`) que LDAP, SMTP, OIDC et les proxys peuvent joindre bien que privées. Sinon les adresses privées, loopback et link-local sont refusées. Le SMTP sans chiffrement n'est accepté que pour ces hôtes. Une plage `/0` ou une faute de frappe arrête le serveur. |
| `TRUSTED_PROXY_IPS` | Non | | Reverse proxies dont le `X-Forwarded-For` est cru (plages CIDR). Une plage `/0` arrête le serveur. |
| `ANONYMOUS_REGISTRY_READS_PER_MINUTE` | Non | `1200` | Lectures anonymes npm et Docker par minute et par client ; `0` supprime la limite. |
| `DB_MAX_CONNECTIONS` | Non | `10` | Taille du pool. |
| `ARTIFERRIS_AUDIT_BACKFILL_FORCE` | Non | `false` | La tâche de remplissage de l'organisation des anciens événements d'audit ne démarre pas sous 3 connexions ; `true` la force. |
| `STORAGE_ROOT` | Non | `./data` | Tarballs npm et blobs Docker. Un volume persistant en production. |
| `BIND_ADDR` | Non | `0.0.0.0:8080` | Adresse d'écoute. |
| `STATIC_DIR` | Non | `./static` | Frontend compilé (hors image fournie). |
| `RUST_LOG` | Non | aucun log | Filtre `tracing`, par exemple `info`. |
| `CORS_ALLOWED_ORIGIN` | Non | toute origine | Restreint le CORS à une origine. À laisser vide quand le frontend est servi par l'API. |
| `ARTIFERRIS_DOCKER_TOKEN_REALM` | Non | dérivé de la requête | Realm des tokens Docker, normalement calculé par requête. À fixer seulement derrière un intermédiaire qui ne transmet pas fidèlement `Host` et le schéma ; il fige alors tous les clients sur ce realm. En `https://` hors localhost. |

Non configurables (codés en dur) : purge des métriques (horaire), de la rétention (6 heures) et des envois Docker
abandonnés (horaire, blobs non référencés après 48 heures). Le scan Docker exécute `trivy`, présent dans l'image
fournie.

## Mises à jour et rotation de la clé de chiffrement

Les secrets stockés sont écrits dans un format versionné que les versions antérieures ne lisent pas. Les valeurs
existantes ne sont réécrites que si `SECRETS_REENCRYPT_LEGACY=true` (Helm : `artiferris.reencryptStoredSecrets`) ; sans
cela, le serveur journalise le nombre de valeurs en attente.

**Mettre à jour** : sauvegardez la base, déployez sans `SECRETS_REENCRYPT_LEGACY`, vérifiez SSO, e-mails et MFA, puis
positionnez-la à `true` et redémarrez. La ligne `re-encrypted stored secrets` donne le nombre de valeurs converties.

**Revenir en arrière** demande de restaurer la base : cette version applique les migrations 0006 à 0010, et la
précédente (0.4.6 et avant) refuse de démarrer sur une base qui enregistre des migrations qu'elle ne connaît pas. `helm rollback` et
`--atomic` ne les annulent pas. À partir de cette version, le serveur ignore les migrations d'une version plus
récente, ce qui ne fonctionne que si le changement de schéma était additif (les notes de version le disent). Les
versions antérieures ne lisent pas non plus le nouveau format des secrets : TOTP, SMTP, LDAP, OIDC et proxys cessent
de fonctionner.

**Changer la clé** :
1. Sauvegardez la base.
2. Mettez la nouvelle valeur dans `SECRETS_ENCRYPTION_KEY`, l'ancienne dans `SECRETS_ENCRYPTION_KEY_PREVIOUS` et
   `SECRETS_REENCRYPT_LEGACY=true`. Avec Helm : `--set secrets.secretsEncryptionKey=<nouvelle> --set
   secrets.secretsEncryptionKeyPrevious=<ancienne> --set artiferris.reencryptStoredSecrets=true`.
3. Redémarrez tous les réplicas ensemble : un réplica sur l'ancienne clé ne lit pas ce qu'un autre a déplacé.
4. `re-encrypted stored secrets` confirme ; une erreur signale des valeurs illisibles avec les deux clés, laissées
   telles quelles.
5. Retirez `SECRETS_ENCRYPTION_KEY_PREVIOUS` (Helm : retirez `secretsEncryptionKeyPrevious` et relancez un upgrade).

Une valeur illisible (mauvaise clé, après une restauration) est journalisée avec son type et son propriétaire, et les
réglages SSO et SMTP renvoient `secret_unreadable: true`. Ce qui en dépend reste en panne jusqu'à la bonne clé ou à
une nouvelle saisie.

## Sécurité

- Mots de passe en Argon2 ; MFA obligatoire ; un token distinct et de courte durée pour « mot de passe vérifié, second
  facteur attendu », non interchangeable avec une session.
- Droits par dépôt vérifiés sur chaque route, pas seulement masqués dans l'interface.
- Journaux d'audit (365 jours par défaut, `AUDIT_RETENTION_DAYS`, `0` pour tout garder) et de sécurité. Les échecs de
  connexion enregistrent le nom saisi (64 caractères au plus) : un mot de passe collé dans ce champ peut s'y trouver,
  lisible des super-admins.
- Les corps de requête ne sont lus qu'après les contrôles de dépôt, de rôle et de portée. Les blobs Docker vont sur
  disque au fil de l'eau ; les documents gardés en mémoire puisent dans un budget de 1 Gio (`503` s'il est épuisé) et un
  client ou utilisateur n'a que quatre corps en cours (`429`).
- Les tokens d'accès Docker durent 2 minutes. Lire à travers un groupe exige `read` sur chaque membre.
- Contenu public mis en cache 1 an par digest (blobs et manifests Docker), 1 jour pour les archives npm ; un contenu
  privé est `private, no-store`. Un CDN peut garder un blob après le passage d'un dépôt en privé : mettez le registre
  derrière un cache qui respecte les purges, ou ne cachez pas `/v2/` en bordure.
- Les réponses `/api` sont `no-store` et `Vary: Authorization`. Toute réponse porte `Permissions-Policy` et
  `Cross-Origin-Opener-Policy: same-origin`.
- Une requête JSON dont le corps reste bloqué 30 s ou dure plus de 120 s reçoit `408`. Les connexions et délais en
  bordure restent à la charge de l'ingress.
- `npm audit` depuis l'interface : 30 exécutions par minute et par compte, résultats gardés 10 minutes. Le scan des
  dépendances envoie des noms et versions de paquets à `registry.npmjs.org` ; ne pointez pas `npm audit` sur
  ArtiFerris si ces noms ne doivent pas sortir de votre réseau.
- Un proxy n'envoie ses identifiants qu'au distant configuré (même schéma, hôte et port, en `https`) ; ailleurs, il
  tire anonymement.
- Les téléchargements sont comptés par client (IPv4, /64 IPv6) une fois par paquet et par heure, en mémoire.
  Derrière un reverse proxy, définissez `TRUSTED_PROXY_IPS`.
- Le trafic anonyme a un budget par client (IPv4, /64 IPv6) et par minute : pages et API publiques, lectures npm et Docker
  (`ANONYMOUS_REGISTRY_READS_PER_MINUTE`, 1 200 par défaut, `0` pour supprimer la limite ; `429` avec `Retry-After`). Une
  requête avec un jeton n'est jamais comptée. Les réplicas partagent ces compteurs par la base, toutes les deux secondes :
  un client peut dépasser la limite d'au plus ce qui arrive entre deux synchronisations, et si la base ne répond pas chaque
  réplica limite seul. Postgres ne reçoit que des empreintes (hachage à clé), jamais une adresse. Ce budget n'arrête pas une
  inondation distribuée : mettez une limite à l'ingress ou un CDN devant. Les connexions échouées ont leur propre limite,
  par processus.
- Les fichiers de marque sont validés par leur signature, jamais par leur `Content-Type`.
- Le jeton d'activation voyage dans le fragment du lien (`/activate#token=…`), qu'aucun serveur ni journal d'accès ne
  voit ; la page le retire ensuite de la barre d'adresse. Il est à usage unique côté serveur et expire. Au retour d'un
  SSO, le navigateur n'accepte le token de session que s'il a lancé la connexion dans les 10 dernières minutes, en plus
  du cookie et du `state` que le serveur vérifie.

Les README de paquets sont assainis côté serveur, mais leurs images peuvent venir de n'importe quel hôte `https` : sur
les pages publiques, chaque visiteur révèle donc son IP et son User-Agent à cet hôte. C'est un risque accepté ; pour le
réduire, restreignez `img-src` dans `crates/artiferris-api/src/main.rs`.

Une faille de sécurité ? Signalez-la en privé, sans ouvrir d'issue publique.

## Référencement (SEO)

Les pages publiques du catalogue (`/explorer`, `/artiferris-npm`, `/artiferris-docker`, `/@utilisateur`,
`/o/<organisation>` et leurs dépôts, paquets et images) ont leur propre `<head>` : titre, description, URL canonique,
Open Graph, carte Twitter et données schema.org pour les paquets. Le corps reste rendu par le navigateur.

**L'indexation est désactivée par défaut.** Un super-admin l'active dans Administration, Paramètres de l'organisation
publique. Désactivée, toutes les pages envoient `noindex`, `robots.txt` interdit tout et le plan du site est vide.
Activée, `/robots.txt` interdit l'API, les registres et l'application, et `/sitemap.xml` (fichiers de 40 000 URL, en
cache 10 minutes, 500 000 URL au plus) liste les pages publiques. Les pages de recherche (`?q=`) restent en `noindex`.

**Langue** : celle de l'en-tête `Accept-Language` (première langue traduite par ordre de préférence ; `fr-CA` donne
le français), l'anglais sinon, donc pour un robot. L'URL est la même dans toutes les langues : la balise canonique ne
change pas, la réponse porte `Vary: Accept-Language`, et `<html lang>`, `og:locale` et `inLanguage` suivent.

**Bloquer par organisation et pour l'instance** (Administration, Paramètres système) :
- « Page publique » : fermée, les pages de l'organisation répondent comme si rien n'était publié (absentes de la
  recherche, des suggestions, des compteurs et du plan du site). Sur l'organisation publique, elle ferme le catalogue de
  toute l'instance (super-admin) : `robots.txt` interdit tout et le plan du site est vide.
- « Bloquer les moteurs de recherche » : les pages restent visibles mais envoient `noindex, nofollow` et quittent le
  plan du site.

Les deux sont ouverts par défaut. L'interrupteur d'indexation de l'instance reste prioritaire. Les dépôts personnels
(`/@utilisateur`) suivent les réglages de l'instance.

Le plan du site est reconstruit par une seule requête à la fois (la copie précédente sert entretemps, et si la
reconstruction échoue). Le réglage d'indexation est relu toutes les 30 secondes au plus ; sans valeur connue,
`robots.txt` et le plan répondent `503`. Un `<head>` qui met plus de 300 ms est remplacé par le générique. Les URL
canoniques viennent de `PUBLIC_URL`. Un dépôt privé, inconnu ou supprimé reçoit le `<head>` générique, sans rien révéler.

## API publique

**Authentification**
- `POST /api/auth/login` : identifiants, puis réponse d'inscription MFA.
- `POST /api/auth/register` : auto-inscription dans l'organisation publique (400 ailleurs), même réponse.

**Détails d'un paquet ou d'une image** (60 requêtes par minute et par IP pour un anonyme, `429` au-delà)
- `GET /api/repositories/{id}/packages/npm/{name}` : versions, dist-tags, `downloads_7d`, `readme_html` (128 Ko lus,
  nettoyé ; `null` sans README) et `registry_url`.
- `GET /api/repositories/{id}/packages/docker/{image}` : `downloads_7d`, tags (`size_bytes`, `null` pour un index
  multi-architecture) et `image_reference`.

Les téléchargements comptent un `GET` d'archive npm ou de manifest Docker par tag sur un dépôt hosted (ni `HEAD`, ni
digest, ni proxy ou groupe), écrits toutes les 30 secondes, par jour, sans donnée sur l'utilisateur, purgés après 13
mois. Les chiffres sont indicatifs.

**Catalogue public** : sans authentification, `Cache-Control: no-store`, 60 requêtes par minute et par IP (`429` avec
`Retry-After`). Les listes d'accueil et les compteurs sont en cache 30 secondes : un dépôt passé en privé peut y
rester listé aussi longtemps, son contenu restant protégé.
- `GET /api/public/catalogs` : un catalogue par format (`format`, `name`, `label`, `entry_count`).
- `GET /api/public/search` : paramètres `q` (100 caractères au plus), `format` (`npm`, `docker`), `owner`
  (`personal:<utilisateur>`, `organization:<slug>`), `sort` (`relevance`, `updated`, `popular`), `page` (1 à 100),
  `per_page` (1 à 50, 20 par défaut). Tolère les fautes de frappe dès 3 caractères (`match_kind: fuzzy`, en dernier).
  Chaque résultat porte `downloads_7d`, le propriétaire, le dépôt et l'URL d'installation.
- `GET /api/search` : la même recherche pour un utilisateur connecté, sur tout ce qu'il peut lire (proxys compris, avec
  ce qu'ils ont en cache) ; chaque `repository` ajoute `id` et `repo_type`. Un super-admin cherche partout. `401` sans
  token.
- `GET /api/public/suggest` : `q` de 2 à 100 caractères, 8 résultats au plus (nom exact, préfixe, sous-chaîne, puis
  approximatif) ; filtres `format`, `owner` et `limit`. Budget propre : 120 requêtes par minute et par IP.
- `GET /api/public/owners/{kind}/{slug}` : nom et nombres de dépôts, paquets et images. `404` sans dépôt public,
  réponse identique pour un propriétaire inexistant.
- `GET /api/repositories/by-org/{slug}/{repo_name}` : un dépôt public d'organisation (`404` sinon), pendant de
  `by-owner`. Un anonyme reçoit le dépôt sans `quota_bytes`, `retention_keep_last_n` ni `organization_id`.

## Feuille de route

- [ ] SAML.
- [ ] Autres formats : Maven/Gradle, PyPI, NuGet, Cargo, Go, Helm, dépôts raw.
- [ ] Stockage objet compatible S3 derrière `StorageBackendPort`, pour plusieurs réplicas (aujourd'hui un volume, un
      réplica).
- [ ] Haute disponibilité et réplication géographique.
- [ ] Signature et provenance des paquets (Sigstore, npm provenance).
- [ ] Espaces de noms par utilisateur dans une organisation (scopes `@user/…`).
- [ ] Limitation de débit du trafic anonyme ; distribution par CDN.
- [x] Recherche et découverte publiques de paquets.

## Site web

Le site public, <https://www.artiferris.pro>, vit dans [`website/`](website). C'est un projet Angular à part (il ne partage
rien avec `frontend/`), prérendu en fichiers statiques et servi par nginx. L'application est sur <https://app.artiferris.pro> ;
`artiferris.pro` redirige vers `www`.

- **Pages et langues.** Accueil, Produit, Registres, Installation, Sécurité et Feuille de route, en anglais (par défaut),
  français, italien, espagnol et allemand : trente pages prérendues sous `/<langue>/`. La racine `/` n'est pas une page : nginx
  redirige vers la langue de l'en-tête `Accept-Language` du visiteur, ou vers `/en/`.
- **Développement local.** Node 26 :

  ```bash
  cd website
  npm ci
  npm start        # serveur de développement sur http://localhost:4200
  npm run lint
  npm test
  npm run build    # site prérendu dans dist/artiferris-website/browser, vérifié ensuite par scripts/check-dist.mjs
  ```

- **Ressources générées.** Elles sont commitées, un build normal ne les régénère pas (commandes lancées depuis `website/`) :
  - `npm run images` : logos, favicon, icône Apple et images de partage, une par langue, à partir de `frontend/public/Logo.png` ;
  - `npm run plan` : le dessin d'architecture, `src/app/shared/architecture-plan/architecture-plan.html` ;
  - `node scripts/capture-screens.mjs` : les captures du produit dans `public/images/screens/`, tirées du Storybook de
    l'application ; le mode d'emploi est en tête du script.
- **Image.** `website/Dockerfile` construit le site et le sert avec nginx, en utilisateur sans privilège sur le port 8080
  (système de fichiers en lecture seule, `/tmp` seulement). La Content-Security-Policy est générée à partir des pages
  construites. L'image est `masmarino/artiferris-website` :

  ```bash
  docker build -t artiferris-website website/
  docker run --rm --read-only --tmpfs /tmp -p 8080:8080 artiferris-website   # http://localhost:8080
  ```

- **Chart.** [`website/helm/artiferris-website`](website/helm/artiferris-website) déploie un pod sans état, un `Service`, un
  `Ingress` pour `www.artiferris.pro` et `artiferris.pro` et une `NetworkPolicy`. Le certificat vient par défaut de l'émetteur
  cert-manager `artiferris-dns01-issuer` (DNS-01), celui de l'application ; l'`Ingress` porte une priorité Traefik qui passe
  devant le joker `*.artiferris.pro` de l'application, sans quoi `www` serait pris pour une organisation.
- **Déploiement.** Seulement avec les tags de version, avec l'application : la release est `artiferris-website`, dans le
  namespace `artiferris-website`.
- **DNS.** `www.artiferris.pro` et `artiferris.pro` doivent pointer uniquement sur le load balancer Traefik du cluster.
- **Aucun pistage.** Le site ne pose aucun cookie et ne charge rien d'une autre origine.

## Licence

ArtiFerris est distribué sous [licence Apache 2.0](LICENSE).
