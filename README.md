*[Read this in English](README.en.md)*

# ArtiFerris

Gestionnaire d'artefacts auto-hébergé — un registre npm et un registre
Docker/OCI derrière une seule console d'administration, un seul système
d'authentification, et un seul jeu de contrôles de gouvernance (quotas,
rétention, RBAC, audit).

Backend Rust en architecture hexagonale/DDD (`artiferris-domain` →
`artiferris-application` → `artiferris-infrastructure`/`artiferris-api`, plus les
crates adaptateurs de protocole `artiferris-npm` et `artiferris-docker`), avec un
frontend Angular 22 servi par le même binaire.

## Sommaire

- [Fonctionnalités](#fonctionnalités)
- [Architecture](#architecture)
- [Stack technique](#stack-technique)
- [Lancer le projet en local](#lancer-le-projet-en-local)
- [Déploiement](#déploiement)
- [Référence de configuration](#référence-de-configuration)
- [Sécurité](#sécurité)
- [Comparaison avec les alternatives](#comparaison-avec-les-alternatives)
- [Feuille de route](#feuille-de-route)
- [Licence](#licence)

## Fonctionnalités

**Registres**
- Protocole de registre npm (publish, install, unpublish, dist-tags)
- Protocole de registre Docker/OCI (push, pull, suppression de manifest)
- Trois types de dépôt par format : **hosted** (contenu que vous hébergez),
  **proxy** (cache transparent devant un registre amont, ex. npmjs.org ou
  Docker Hub), **group** (agrège plusieurs dépôts derrière un seul point
  d'entrée)
- Quotas de stockage par dépôt (côté Docker, ce que le dépôt détient : les
  blobs référencés par ses manifests, ceux envoyés mais pas encore référencés,
  les corps de manifests, 1 Kio par tag et les octets des envois en cours ;
  32 envois ouverts au plus par dépôt et 10 000 tags). Un paquet npm contient
  au plus 5 000 versions et 32 Mio de manifests de versions
- Politique de rétention par dépôt (conserver les N dernières
  versions/étiquettes par paquet/image ; les références taguées comme
  `latest` ne sont jamais purgées) — purge automatique toutes les 6 heures.
  Les tags Docker sont classés selon la date à laquelle le tag lui-même a été
  posé : remettre un tag sur une ancienne image (un rollback) en fait le plus
  récent. Dans un dépôt avec politique, les manifests que plus aucun tag ni
  index ne référence sont aussi supprimés 7 jours après que leur dernier tag a
  été déplacé (ou après leur push s'ils n'ont jamais eu de tag)
- Republier une version npm dépubliée est refusé, quels que soient ses octets
- Navigateur de paquets/images avec vue détaillée par paquet : README npm
  (Markdown converti puis nettoyé côté serveur, liens en nouvel onglet, images
  en https uniquement), commande d'installation, taille des tags Docker
- Catalogues publics `artiferris-npm` et `artiferris-docker` (`/artiferris-npm`,
  `/artiferris-docker`), sans authentification : recherche dans tous les
  paquets/images des dépôts publics hosted, personnels et d'organisation. Un
  catalogue ne sert pas à installer : chaque résultat renvoie vers l'URL de son
  propriétaire. Pages de profil `/@utilisateur` et `/o/<organisation>`. Le
  préfixe `artiferris-` est réservé (dépôts, utilisateurs, organisations).
  Référencement par les moteurs de recherche (désactivé par défaut)

**Scan de sécurité**
- Audit des dépendances npm contre la base d'avis publique, déclenché
  automatiquement à chaque publication
- Scan de vulnérabilités des images Docker via
  [Trivy](https://github.com/aquasecurity/trivy), déclenché automatiquement
  à chaque push, plus relance manuelle

**Multi-tenant**
- Organisations résolues par sous-domaine (`acme.artiferris.example` route vers
  l'organisation `acme`), chacune avec ses propres dépôts, utilisateurs et
  marque
- Admins d'organisation aux droits scopés à leur seule organisation ; un
  super-admin peut cibler n'importe quelle organisation via
  `?organization_id=`
- Organisation publique par défaut pour les déploiements mono-tenant

**Authentification & contrôle d'accès**
- Comptes locaux avec hachage de mot de passe Argon2
- SSO : LDAP/Active Directory et OIDC (OpenID Connect), en plus des comptes
  locaux — SAML n'est pas encore supporté
- Double authentification obligatoire : TOTP ou clé d'accès (WebAuthn/
  passkey), avec codes de secours à usage unique
- Tokens API personnels (portée par utilisateur ; les admins peuvent lister
  et révoquer les tokens de tous les utilisateurs)
- RBAC par dépôt (`read` / `write` / `admin`), attribué par utilisateur
- Invitations de compte par e-mail et parcours d'activation
- Limitation des tentatives de connexion échouées (par processus)

**Console d'administration**
- Métriques d'usage, historique horodaté (snapshots horaires), page d'état
  de santé
- Journal d'audit et journal des événements de sécurité
- Gestion des utilisateurs (création, suppression, promotion super-admin)
- Paramètres SMTP (hôte/port/identifiants/sécurité/nom et adresse
  d'expéditeur) avec bouton d'envoi d'e-mail de test
- Marque personnalisée : remplacer le logo/favicon par défaut partout
  (interface + e-mails), pour du marque blanche ou des déploiements en
  cluster isolé
- Export/import de configuration pour sauvegarde et restauration, instances
  mono-organisation uniquement. L'export refuse une instance dont des
  utilisateurs ont des dépôts personnels (le fichier n'a pas de propriétaire
  pour un dépôt). Le fichier peut peser jusqu'à 32 Mio et lister au plus
  100 000 utilisateurs, dépôts et permissions chacun ; les e-mails d'invitation
  partent après la validation, huit à la fois. L'import tient dans une seule
  transaction : les entrées qu'il
  ne peut pas restaurer sont listées et ignorées, le reste est écrit d'un bloc
  ou pas du tout, un import échoué peut donc être relancé
- E-mails transactionnels HTML (compte créé, mot de passe régénéré,
  MFA/passkey ajouté) avec la marque du déploiement intégrée en ligne (CID,
  donc affichée même quand les images externes sont bloquées)

## Architecture

Hexagonale/DDD, les dépendances pointent vers l'intérieur :

| Crate | Rôle |
|---|---|
| `artiferris-domain` | Entités, objets de valeur, ports (traits) — aucune dépendance vers un framework ou une I/O |
| `artiferris-application` | Cas d'usage, orchestrant la logique métier via les ports |
| `artiferris-infrastructure` | Implémentations des ports : Postgres, stockage fichier, SMTP, Trivy, Argon2, JWT |
| `artiferris-api` | Serveur HTTP Axum, handlers/DTO des routes, assemble tout, sert le frontend compilé |
| `artiferris-npm` | Adaptateur du protocole de registre npm (son propre jeu de routes, monté dans `artiferris-api`) |
| `artiferris-docker` | Adaptateur du protocole de registre Docker/OCI (même principe) |

Les dépôts (`PackageRepository`) et les permissions sont en event-sourcing ;
le reste de l'état (utilisateurs, paramètres, journal d'audit, métriques)
est du CRUD classique sur Postgres.

## Stack technique

- **Backend :** Rust (édition 2024), [Axum](https://github.com/tokio-rs/axum), [SQLx](https://github.com/launchbadge/sqlx) + Postgres, [webauthn-rs](https://github.com/kanidm/webauthn-rs), [lettre](https://github.com/lettre/lettre)
- **Frontend :** Angular 22, composants standalone, signals (pas de NgRx)
- **Stockage :** système de fichiers local (`StorageBackendPort` est abstrait, mais seule une implémentation fichier existe aujourd'hui — voir la [feuille de route](#feuille-de-route))

## Lancer le projet en local

```bash
git clone <ce-repo>
cd hangar
cp .env.example .env   # renseigner POSTGRES_PASSWORD / JWT_SECRET
./scripts/dev.sh
```

Ceci démarre Postgres via `docker-compose`, applique les migrations, puis
lance le backend (`cargo run -p artiferris-api`, port 8081) et le frontend
(`ng serve`, port 4200, hot reload) en parallèle. `scripts/dev.sh` fixe
`ARTIFERRIS_BOOTSTRAP_ADMIN_USERNAME=admin` /
`ARTIFERRIS_BOOTSTRAP_ADMIN_PASSWORD=admin123`, donc vous pouvez vous connecter
immédiatement.

Lancer les tests :

```bash
cargo test --workspace                  # backend
npm test --prefix frontend              # frontend
```

## Déploiement

### Docker Compose

Un déploiement mono-nœud (ArtiFerris + Postgres) tient en une commande :

```bash
cp .env.example .env   # renseigner POSTGRES_PASSWORD, JWT_SECRET, SECRETS_ENCRYPTION_KEY, PUBLIC_URL, ARTIFERRIS_BOOTSTRAP_ADMIN_*
docker compose up -d --build
```

Le serveur refuse de démarrer si `JWT_SECRET` ou `SECRETS_ENCRYPTION_KEY` fait
moins de 32 octets ou commence par `change-me`, si
`ARTIFERRIS_BOOTSTRAP_ADMIN_PASSWORD` est encore le placeholder, ou si
`PUBLIC_URL` pointe sur `0.0.0.0`. Générez les secrets avec
`openssl rand -base64 48`. `POSTGRES_PASSWORD` se retrouve tel quel dans
`DATABASE_URL` : n'y mettez que des caractères sûrs dans une URL
(`openssl rand -hex 24`).

Le health check du conteneur appelle `/readyz` (la base répond, sans attendre
une connexion libre : un pool entièrement occupé compte comme disponible tant
qu'une vérification récente a réussi) ; `/healthz` dit seulement que le
processus tourne.

Voir la [référence de configuration](#référence-de-configuration)
ci-dessous pour chaque variable câblée par `docker-compose.yml`. Le realm de
jeton du protocole Docker (`ARTIFERRIS_DOCKER_TOKEN_REALM`) est dérivé
automatiquement par requête et ne nécessite normalement aucune
configuration — à définir explicitement uniquement si ce déploiement se
trouve derrière quelque chose qui ne transmet pas fidèlement le `Host` et le
schéma d'origine.

### Kubernetes (Helm)

```bash
helm upgrade --install artiferris ./helm/artiferris \
  --namespace artiferris --create-namespace \
  --set image.tag=<tag de release> \
  --set ingress.host=app.example.com --set ingress.wildcardHost='*.example.com' \
  --set artiferris.baseDomain=example.com
```

- `image.tag` n'a pas de valeur par défaut : choisissez une release.
  `image.digest` épingle l'image par digest (c'est ce que fait la CI). Un tag
  `latest` est toujours retiré du registre.
- Le chart génère une fois le mot de passe de la base, `JWT_SECRET` et
  `SECRETS_ENCRYPTION_KEY` dans `<release>-secrets`, et les retrouve à la mise
  à jour avec `lookup` de Helm. `lookup` ne renvoie rien avec `helm template`,
  `--dry-run`, Argo CD ou Flux : avec ces outils, renseignez vous-même
  `secrets.postgresPassword`, `secrets.jwtSecret` et
  `secrets.secretsEncryptionKey`, sinon chaque rendu en invente de nouveaux et
  tous les secrets stockés deviennent illisibles. Le Secret porte
  `helm.sh/resource-policy: keep` : `helm uninstall` le laisse en place. Les
  deux PersistentVolumeClaims (données du registre et base) sont conservés de la
  même façon tant que `persistence.keepOnUninstall` vaut `true` (défaut) ;
  supprimez-les à la main pour effacer les données, ou mettez `false` pour que
  `helm uninstall` les supprime. Un
  `secrets.secretsEncryptionKey` que vous passez remplace celui du Secret
  (c'est ainsi qu'une rotation fournit la nouvelle clé) : ne le repassez pas
  aux mises à jour suivantes sauf pour le changer.
- Les pods tournent en uid 100 / gid 101 (les ids fixés par l'image), sans
  élévation de privilèges ni capabilities, avec un système de fichiers racine
  en lecture seule et le profil seccomp `RuntimeDefault`. Une NetworkPolicy
  n'autorise que le pod ArtiFerris à joindre Postgres
  (`networkPolicy.enabled` ; nécessite un CNI qui les applique).
- L'ingress utilise les middlewares Traefik de `ingress.middlewares`
  (`traefik-https`, `traefik-headers`, `traefik-ratelimit`, tous dans le
  namespace `traefik` par défaut). Ils doivent déjà exister : le chart ne les
  crée pas. Une valeur vide n'en utilise aucun.
- Renseignez `artiferris.trustedProxyIps` avec le réseau de pods du contrôleur
  d'ingress, voir `TRUSTED_PROXY_IPS`. Avec l'ingress activé, le chart refuse
  de se rendre tant que la valeur est vide (tous les visiteurs partageraient un
  seul compteur de tentatives de connexion et un seul budget de requêtes du
  catalogue public) ; `artiferris.allowSharedThrottleBucket=true` l'accepte.
  Le serveur journalise aussi un avertissement, au plus une fois par heure,
  quand des requêtes portent `X-Forwarded-For` depuis une adresse privée alors
  que `TRUSTED_PROXY_IPS` est vide.
- Un `startupProbe` laisse cinq minutes à un démarrage lent (migrations,
  vérification des secrets) avant que la sonde de vivacité ne prenne le relais.
- Le déploiement CI lance `helm upgrade --install --atomic --cleanup-on-fail` :
  une mise à jour en échec ramène la release à la révision précédente. Les
  migrations déjà exécutées restent appliquées : après le premier déploiement de
  la version qui a introduit les migrations 0006 à 0010, l'image précédente ne
  démarre pas sur la base, donc la release restaurée ne devient pas prête non
  plus et il faut restaurer la sauvegarde (voir « Mettre à jour et faire
  tourner » plus bas).

## Référence de configuration

Chaque variable lue par `artiferris-api` depuis son environnement.
`docker-compose.yml` câble déjà celles nécessaires à un déploiement
mono-nœud ; ce tableau fait référence pour un déploiement conteneur nu ou
pour surcharger les valeurs par défaut.

| Variable | Obligatoire | Défaut | Description |
|---|---|---|---|
| `DATABASE_URL` | **Oui** | — | Chaîne de connexion Postgres, ex. `postgres://user:pass@host:5432/artiferris`. |
| `JWT_SECRET` | **Oui** | — | Signe les tokens de session et les tokens d'accès au registre Docker. Au moins 32 octets aléatoires (`openssl rand -base64 48`) ; une valeur plus courte ou commençant par `change-me` arrête le serveur au démarrage. Le faire tourner invalide toutes les sessions et tous les `docker login`. |
| `SECRETS_ENCRYPTION_KEY` | **Oui** | — | Chiffre ce que la base stocke sous forme récupérable : mots de passe SMTP, secrets LDAP et OIDC, identifiants des dépôts proxy et graines TOTP. Différente de `JWT_SECRET`, au moins 32 octets, ne commençant pas par `change-me`. Elle est dérivée avec HKDF, qui ne renforce pas une valeur faible : elle doit être aléatoire (`openssl rand -base64 48`), pas une phrase de passe. Voir [Faire tourner `SECRETS_ENCRYPTION_KEY`](#faire-tourner-secrets_encryption_key). |
| `SECRETS_ENCRYPTION_KEY_PREVIOUS` | Non | — | Uniquement pendant une rotation : la clé avec laquelle les secrets stockés sont chiffrés actuellement. |
| `SECRETS_REENCRYPT_LEGACY` | Non | `false` | `true` autorise la passe de démarrage à réécrire les secrets stockés au format courant et, après une rotation, sous la nouvelle clé. Désactivé par défaut car la version précédente ne sait pas lire les valeurs réécrites : voir [Faire tourner `SECRETS_ENCRYPTION_KEY`](#faire-tourner-secrets_encryption_key). |
| `ARTIFERRIS_SSRF_ALLOWED_CIDRS` | Non | — | Adresses ou plages CIDR séparées par des virgules (`10.20.0.0/16,192.168.1.5`) vers lesquelles les hôtes LDAP, SMTP, OIDC et des dépôts proxy peuvent résoudre bien qu'elles soient privées. Sans elle, toute adresse privée, loopback ou link-local est refusée, ce qui exclut un annuaire ou un relais mail interne. L'émetteur OIDC et chaque endpoint de son document de découverte doivent être en `https` et passer le même contrôle. Le SMTP non chiffré (`security: none`) n'est accepté que pour les hôtes de cette liste. Une faute de frappe arrête le serveur au démarrage, tout comme une plage `/0` (elle désactiverait le contrôle). |
| `TRUSTED_PROXY_IPS` | Non | — | Adresses ou plages CIDR, séparées par des virgules, des reverse proxies dont le `X-Forwarded-For` est cru. Une faute de frappe arrête le serveur au démarrage, tout comme une plage `/0` (elle ferait confiance à tout Internet). |
| `DB_MAX_CONNECTIONS` | Non | `10` | Taille du pool de connexions Postgres ; une connexion reste toujours ouverte. Une valeur qui n'est pas un nombre positif arrête le serveur au démarrage. |
| `ARTIFERRIS_AUDIT_BACKFILL_FORCE` | Non | `false` | La tâche qui renseigne l'organisation des anciens événements d'audit ne démarre pas avec `DB_MAX_CONNECTIONS` inférieur à 3 (elle se disputerait le pool avec les requêtes). `true` la lance quand même. |
| `ARTIFERRIS_BASE_DOMAIN` | **Oui** | — | Domaine de base par rapport auquel les organisations sont résolues en sous-domaines (ex. `artiferris.example` pour que `acme.artiferris.example` résolve l'organisation `acme`). Aucun fallback : un déploiement mal configuré doit échouer au démarrage plutôt que de router silencieusement tous les sous-domaines vers l'organisation publique. |
| `STORAGE_ROOT` | Non | `./data` | Chemin du système de fichiers où sont stockés les tarballs npm et les blobs Docker. Doit être un volume persistant dans tout déploiement réel. |
| `BIND_ADDR` | Non | `0.0.0.0:8080` | Adresse/port sur lequel le serveur HTTP écoute. |
| `STATIC_DIR` | Non | `./static` | Chemin des assets frontend compilés servis pour les routes non-API. Pertinent uniquement si vous n'utilisez pas l'image Docker fournie. |
| `RUST_LOG` | Non | — (aucun log sans elle) | Filtre `tracing_subscriber`, ex. `info` ou `artiferris_api=debug,info`. Sans elle, le conteneur ne log quasiment rien. |
| `CORS_ALLOWED_ORIGIN` | Non | permissif (toute origine) | Restreint le CORS à une seule origine. À laisser vide en dev local (`ng serve` sur un port différent du backend) ou quand le frontend est servi depuis la même origine que l'API (configuration par défaut de l'image fournie). |
| `ARTIFERRIS_DOCKER_TOKEN_REALM` | Non | dérivée par requête à partir du `Host` de cette requête et du schéma de `PUBLIC_URL` | Surcharge l'URL de realm intégrée dans chaque challenge `WWW-Authenticate`, sur laquelle le CLI Docker résout les requêtes de token pour `login`/`push`/`pull` — normalement dérivée automatiquement, donc correcte quel que soit le nombre de sous-domaines d'organisation servis par ce déploiement. À définir uniquement quand le `Host`/schéma de la requête n'est pas fiable (ex. un intermédiaire qui ne les transmet pas fidèlement) ; le faire fige alors tous les clients sur ce seul realm fixe, ce qui casse l'authentification pour tout sous-domaine d'organisation autre que celui vers lequel cet hôte résout. Doit être en `https://` pour tout hôte non-localhost (Docker refuse le `http://` simple sinon). |
| `PUBLIC_URL` | Oui, sauf en développement local | dérivée de `BIND_ADDR` | URL externe de l'instance : base des liens d'invitation, schéma du realm de token Docker et de HSTS, origine des passkeys, URL canoniques. Le serveur refuse de démarrer si elle résout vers `0.0.0.0` sur un domaine de base qui n'est pas `localhost` ou `*.localhost`. |
| `ARTIFERRIS_BOOTSTRAP_ADMIN_USERNAME` | Non | — | Nom d'utilisateur du compte créé automatiquement **uniquement si la table `users` est vide**. Peut rester défini au fil des redémarrages/mises à jour. |
| `ARTIFERRIS_BOOTSTRAP_ADMIN_PASSWORD` | Non, mais il faut *un* moyen d'obtenir un premier admin | — | Mot de passe de ce même compte bootstrap. Doit faire ≥ 8 caractères — une valeur plus courte échoue silencieusement (loggé, non fatal) et laisse le déploiement sans admin. Une valeur commençant par `change-me` (le placeholder de `.env.example`) arrête le serveur au démarrage. |

`PUBLIC_URL` retombe sur une URL devinée à partir de `BIND_ADDR`, ce qui
n'est correct qu'en développement local — à définir explicitement dans tous
les autres cas. Elle fournit aussi le schéma (`http`/`https`) qu'utilise la
dérivation automatique par requête d'`ARTIFERRIS_DOCKER_TOKEN_REALM`.

### Mettre à jour et faire tourner `SECRETS_ENCRYPTION_KEY`

Les secrets stockés (identifiants SMTP, LDAP, OIDC et proxy, graines TOTP) sont
écrits dans un format versionné que les versions antérieures ne savent pas
lire. Une valeur enregistrée après la mise à jour l'utilise déjà, et la
lecture accepte les deux formats, mais les valeurs existantes ne sont
réécrites que si `SECRETS_REENCRYPT_LEGACY=true` (Helm :
`artiferris.reencryptStoredSecrets`). Sans cela, le serveur journalise un
avertissement avec le nombre de valeurs en attente et ne modifie rien.

Pour mettre à jour :

1. Sauvegardez la base.
2. Déployez la nouvelle version sans `SECRETS_REENCRYPT_LEGACY`.
3. Vérifiez que le SSO, les mails et le MFA fonctionnent.
4. Positionnez `SECRETS_REENCRYPT_LEGACY=true` et redémarrez. La ligne de log
   `re-encrypted stored secrets` donne le nombre de valeurs converties. La
   variable peut rester : sans rien à convertir, elle ne fait rien.

**Revenir à l'image précédente après le premier démarrage de cette version
demande une restauration de la base.** Ce démarrage applique les migrations
0006 à 0010, et la version précédente (0.4.6 et antérieures) refuse de démarrer
sur une base qui enregistre des migrations qu'elle ne connaît pas (« failed to
run migrations »), que `SECRETS_REENCRYPT_LEGACY` ait été activé ou non.
`helm rollback`, `--atomic` et redéployer l'ancien tag n'annulent pas les
migrations : sauvegardez avant la mise à jour et restaurez cette sauvegarde
pour revenir en arrière. À partir de cette version, le serveur ignore les
migrations enregistrées par une version plus récente, donc revenir à celle-ci
démarre ; elle tourne alors sur le schéma plus récent, ce qui ne fonctionne que
si ce changement de schéma était additif (les notes de version le disent quand
ce n'est pas le cas).

Le format des secrets est une seconde limite : les versions antérieures à
celle-ci ne lisent pas le nouveau format, donc les graines TOTP (les connexions
MFA échouent), les identifiants SMTP, LDAP, OIDC et proxy cessent de
fonctionner. Cela vaut pour toute valeur enregistrée par cette version, et pour
les valeurs existantes dès que `SECRETS_REENCRYPT_LEGACY` les réécrit.

Pour changer la clé :

1. Sauvegardez la base.
2. Mettez dans `SECRETS_ENCRYPTION_KEY` la nouvelle valeur aléatoire, dans
   `SECRETS_ENCRYPTION_KEY_PREVIOUS` la valeur utilisée jusqu'ici, et
   `SECRETS_REENCRYPT_LEGACY=true`. Avec Helm :
   `--set secrets.secretsEncryptionKey=<nouvelle> --set secrets.secretsEncryptionKeyPrevious=<ancienne> --set artiferris.reencryptStoredSecrets=true`.
3. Redémarrez tous les réplicas ensemble. Un réplica qui tourne encore avec
   l'ancienne clé ne peut pas lire une valeur qu'un autre a déjà déplacée sous
   la nouvelle : un redémarrage progressif avec des clés mélangées n'est pas
   sûr.
4. La ligne de log `re-encrypted stored secrets` confirme le changement ; une
   ligne d'erreur signale des valeurs illisibles avec les deux clés, laissées
   telles quelles.
5. Retirez `SECRETS_ENCRYPTION_KEY_PREVIOUS` (avec Helm, retirez
   `secretsEncryptionKeyPrevious` et relancez un upgrade : le chart la supprime
   du Secret et garde la nouvelle clé).

Une valeur illisible avec les clés du serveur (clé différente de celle qui l'a
chiffrée, par exemple après une restauration) est journalisée au niveau erreur
avec son type et l'organisation ou l'utilisateur concerné, et les endpoints de
réglages SSO et SMTP renvoient `secret_unreadable: true`. Tout ce qui en dépend
(connexion SSO, mails, vérifications TOTP, identifiants proxy) reste en panne
tant que la bonne clé n'est pas fournie ou qu'un admin ne ressaisit pas le
secret.

**Non configurable via l'environnement aujourd'hui** (codé en dur) : la
purge des métriques (horaire), la purge de rétention (toutes les 6 heures) et
la purge des envois (horaire), qui supprime les sessions d'envoi expirées et
les blobs Docker qu'aucun manifest n'a référencés dans les 48 heures suivant
leur envoi. Le scanner d'images Docker exécute un binaire `trivy` qui doit
être présent dans le `PATH` (le Dockerfile fourni l'installe ; une image
personnalisée devra l'installer aussi).

## Sécurité

- Hachage de mot de passe Argon2
- MFA obligatoire (TOTP ou WebAuthn/passkey) avec codes de secours à usage
  unique
- Un type de token distinct et de courte durée pour « mot de passe
  vérifié mais pas encore le second facteur », volontairement non
  interchangeable avec un token de session complet
- RBAC par dépôt, vérifié sur chaque route — pas seulement masqué dans
  l'interface
- Journal d'audit et journal des événements de sécurité pour revue admin
- Les corps de requête (envois et manifests Docker, publish, dist-tag et
  unpublish npm) ne sont lus qu'après les contrôles de dépôt, de rôle et de
  portée. Les blobs Docker sont écrits sur disque au fil de l'eau ; les
  documents gardés en mémoire puisent dans un budget partagé de 1 Gio, débité
  au fur et à mesure que le corps arrive, et une requête qui n'y tient pas
  reçoit un `503` ; un client ou un utilisateur qui a déjà quatre corps en
  cours reçoit un `429`
- Les tokens d'accès Docker vivent 2 minutes (`expires_in` l'indique) ; un
  token présenté mais expiré, falsifié ou révoqué reçoit un `401` avec
  challenge, ce qui pousse le client à en demander un nouveau. Lire à travers
  un groupe Docker exige un droit `Read` sur chaque membre, vérifié en direct,
  comme npm
- Les blobs et les manifests par digest des dépôts **publics** sont envoyés
  avec `Cache-Control: public, max-age=31536000, immutable` (le contenu
  derrière un digest ne change jamais) ; les archives npm des dépôts publics
  n'ont que `public, max-age=86400`, car un cache qui garde une copie survit au
  passage du dépôt en privé, et un jour est le compromis. Le contenu d'un dépôt
  privé est `private, no-store` avec `Vary: Authorization`. Un blob gardé par un
  CDN peut donc y rester après le passage d'un dépôt en privé : placez le
  registre derrière un cache qui respecte les purges, ou ne mettez pas `/v2/`
  en cache en bordure, si cela compte
- Toute réponse `/api` est `Cache-Control: no-store` (sauf si le handler a
  choisi le sien, comme le catalogue public et les images de marque) et
  `Vary: Authorization`, puisque son contenu dépend de l'appelant. Toute
  réponse porte aussi `Permissions-Policy` (caméra, micro, géolocalisation,
  paiement, USB et autres capteurs refusés) et
  `Cross-Origin-Opener-Policy: same-origin` (la connexion est une redirection
  pleine page, rien n'utilise `window.opener`) ; l'application, les fichiers
  statiques et l'API JSON ajoutent `Cross-Origin-Resource-Policy: same-origin`,
  mais pas les registres npm et Docker ni les images de marque (d'autres
  clients, des proxys et des aperçus de liens les récupèrent)
- L'API JSON abandonne une requête dont le corps reste bloqué 30 secondes ou
  dure plus de 120 secondes au total (`408`) ; les envois npm et Docker ont
  leurs propres délais de streaming. Le nombre de connexions et les délais en
  bordure restent l'affaire de l'ingress ou du reverse proxy
- `npm audit` depuis l'interface est limité à 30 exécutions par minute et par
  compte connecté, attend au plus 10 secondes l'un de ses 4 emplacements
  sortants (`503` ensuite) et ne fait qu'un appel à npm à la fois par paquet.
  Ses résultats sont gardés 10 minutes, avec une partition à part pour les
  dépôts publics, les seuls que les visiteurs anonymes peuvent lire
- Les dépôts proxy n'envoient leurs identifiants stockés qu'au distant
  configuré (même schéma, même hôte, même port, en `https`) ; une archive ou
  un realm d'authentification sur un autre hôte est récupéré anonymement (un
  proxy Docker Hub avec identifiants tire donc les images publiques sans
  authentification), et un distant qui ferait passer des identifiants en
  `http` clair est refusé
- Les compteurs de téléchargement comptent un client (une adresse IPv4, un /64
  IPv6) une fois par paquet ou image et par heure, en mémoire et jamais
  stockés. Derrière un reverse proxy, définir `TRUSTED_PROXY_IPS` pour que le
  client soit l'adresse transmise
- Le journal d'audit conserve les événements de sécurité et d'administration
  365 jours (`AUDIT_RETENTION_DAYS`, `0` pour tout garder, 36500 au plus) ; un
  balayage cinq minutes après le démarrage puis chaque jour supprime les plus
  anciens. Les événements de paquets, de dépôts et de permissions ne sont
  jamais supprimés. Après une mise à jour depuis une version sans cloisonnement
  du journal par organisation, une tâche de fond renseigne l'organisation des
  anciens événements ; tant qu'elle n'a pas fini (message au démarrage), la vue
  d'audit d'un admin d'organisation ne les liste pas encore. Un seul réplica
  travaille à la fois (un bail qu'il renouvelle après chaque lot ; un autre
  reprend cinq minutes après son arrêt), elle ne garde une connexion du pool que
  le temps d'un lot, et elle ne démarre pas avec `DB_MAX_CONNECTIONS` inférieur
  à 3 sauf `ARTIFERRIS_AUDIT_BACKFILL_FORCE=true`. Chaque ligne renseignée est
  réécrite : sur un long historique, la table des événements et ses index
  grossissent d'environ la moitié jusqu'à ce que l'autovacuum récupère la place
  (mesuré : 2 millions d'événements, environ 13 minutes) Les échecs de
  connexion enregistrent le nom saisi (limité à 64 caractères) : un mot de passe
  collé dans le champ du nom peut donc s'y retrouver, lisible par les
  super-admins pendant la durée de conservation. Ils n'appartiennent à aucune
  organisation (seuls les super-admins les voient), et un refus d'accès est
  rangé sous l'organisation de la personne refusée, pas celle du dépôt visé
- Le scan des dépendances npm et le relais `npm audit` envoient des noms et
  des versions de paquets à `registry.npmjs.org` ; les paquets que ce dépôt
  publie lui-même ne sont jamais cherchés par le scan, et le relais est borné
  en taille. Ne pas pointer `npm audit` sur ArtiFerris si ces noms ne doivent
  pas sortir de votre réseau
- Le jeton d'activation d'un compte voyage dans l'URL (`/activate?token=…`) ; il
  est à usage unique côté serveur (il est supprimé en étant consommé) et expire
- Au retour d'une connexion SSO, le navigateur n'accepte le jeton de session que
  s'il a lui-même lancé la connexion dans les 10 dernières minutes. C'est une
  seconde barrière : le serveur lie déjà le retour à ce navigateur par un cookie
  et un `state`
- Validation par signature de fichier (magic bytes) sur les assets
  téléversés (logo/favicon de marque), sans jamais faire confiance au
  `Content-Type` fourni par le client

Les README de paquets sont assainis côté serveur, mais leurs images peuvent
être chargées depuis n'importe quel hôte `https` (`img-src 'self' data: blob:
https:` dans la CSP). Sur les pages publiques, chaque visiteur révèle donc son
adresse IP et son User-Agent à l'hôte de l'image choisie par l'auteur du
paquet. C'est un risque accepté ; pour le réduire, restreignez `img-src` dans
`crates/artiferris-api/src/main.rs` (par exemple à `'self' data: blob:`, ce
qui bloque les images distantes).

Vous avez trouvé une faille de sécurité ? Merci de la signaler en privé
plutôt que d'ouvrir une issue publique.

## Référence API — Authentification

- **`POST /api/auth/login`** — connecte l'utilisateur avec ses identifiants (nom d'utilisateur + mot de passe), retourne une réponse d'inscription à MFA si la connexion réussit.
- **`POST /api/auth/register`** — auto-inscrit un nouveau compte dans l'organisation publique (désactivée — 400 — sur tout autre sous-domaine d'organisation) ; retourne la même réponse d'inscription à MFA que login.

## Référencement (SEO)

Les pages publiques du catalogue (`/explorer`, `/artiferris-npm`, `/artiferris-docker`, `/@utilisateur`, `/o/organisation` et leurs dépôts, paquets et images) sont servies avec leur propre `<head>` : titre, description, URL canonique, Open Graph, carte Twitter et données structurées schema.org pour les paquets. Le corps des pages reste rendu côté navigateur.

**L'indexation est désactivée par défaut.** Un super-admin l'active dans Administration, Paramètres de l'organisation publique (« Référencement »). Tant qu'elle est désactivée, toutes les pages envoient `noindex`, `robots.txt` interdit tout et le plan du site est vide. Une fois activée, `/robots.txt` interdit l'API, les registres et l'application, et `/sitemap.xml` (index de fichiers de 40 000 URL au plus, mis en cache 10 minutes) liste les propriétaires, dépôts, paquets et images publics, 500 000 au plus. Les pages de résultats de recherche (`?q=`) restent en `noindex`.

Le plan du site est reconstruit par une seule requête à la fois ; les autres reçoivent la copie précédente entre-temps, conservée aussi si une reconstruction échoue. Le réglage d'indexation est relu au plus toutes les 30 secondes et sa dernière valeur connue sert quand la base ne répond pas ; s'il n'y en a jamais eu, `robots.txt` et le plan du site répondent `503` plutôt que de laisser croire que le catalogue est fermé. Un `<head>` qui met plus de 300 ms à se construire est remplacé par le générique : l'application se charge toujours.

Les URLs canoniques et celles du plan du site sont construites à partir de `PUBLIC_URL` : elle doit être l'adresse publique réelle de l'instance. Un dépôt privé, inconnu ou supprimé reçoit le même `<head>` générique qu'une page de l'application, pour ne rien révéler.

**La langue de la page** suit l'en-tête `Accept-Language` de la requête (`fr-CA` donne le français ; la première langue traduite par ordre de préférence l'emporte : en, fr, es, it, de) et, sans en-tête ou sans langue traduite, l'anglais. C'est le cas d'un robot, qui n'en envoie en général pas : l'anglais est donc la langue qu'indexe un moteur de recherche. L'URL, elle, est unique : la balise canonique est la même dans toutes les langues, la réponse porte `Vary: Accept-Language`, et `<html lang>`, `og:locale` et `inLanguage` (données structurées) suivent la langue choisie. Une page générique (application, dépôt privé ou inconnu) ne dit rien dans aucune langue et garde le `lang` du document.

**Bloquer les robots et la page publique, par organisation et pour l'instance.** Dans Administration, Paramètres système d'une organisation, un administrateur de l'organisation peut fermer sa page publique (« Page publique ») : ses pages du catalogue répondent alors comme si rien n'était publié (introuvables pour les visiteurs, absentes de la recherche, des suggestions, des compteurs et du plan du site). Il peut aussi bloquer les moteurs de recherche (« Bloquer les moteurs de recherche pour cette organisation ») : ses pages restent visibles, envoient `noindex, nofollow` et sont absentes du plan du site. Les deux réglages sont ouverts par défaut. Sur l'organisation publique, « Page publique » ferme le catalogue de toute l'instance (réservé à un super-admin) : plus aucune page n'est servie, `robots.txt` interdit tout et le plan du site est vide, comme quand l'indexation est désactivée. L'interrupteur d'indexation de l'instance reste le maître : une organisation ne peut pas s'indexer si l'instance ne l'autorise pas. Les dépôts personnels (`/@utilisateur`) suivent les réglages de l'instance.

## Référence API — Détails d'un paquet ou d'une image

Les téléchargements sont comptés à la volée : un `GET` d'archive npm, ou un `GET` de manifest Docker par tag, servi directement par un dépôt hosted (ni `HEAD`, ni requête par digest, ni dépôt proxy ou groupe). Un client ne compte qu'une fois par paquet ou image et par heure. Les compteurs sont agrégés en mémoire puis écrits toutes les 30 secondes et à l'arrêt, par jour, sans aucune donnée sur l'utilisateur ; les chiffres sont indicatifs. Les jours de plus de 13 mois sont purgés.

- **`GET /api/repositories/{id}/packages/npm/{name}`** — versions, dist-tags, `downloads_7d` (téléchargements des 7 derniers jours), `readme_html` (README de la dernière version, converti et nettoyé côté serveur ; `null` s'il n'y en a pas. Seuls les 128 premiers Ko sont lus, et un README trop imbriqué ou trop chargé en balises est affiché en texte brut échappé) et `registry_url` (le registre du propriétaire, à utiliser pour `npm install`).
- **`GET /api/repositories/{id}/packages/docker/{image}`** — `downloads_7d`, tags (avec `size_bytes` : configuration plus couches, `null` pour un index multi-architecture) et `image_reference` (la référence à tirer, sans tag).

Ces deux routes sont limitées à 60 requêtes par minute et par IP pour un appelant anonyme (429 au-delà) ; un utilisateur connecté n'est pas limité ici.

## Référence API — Catalogue public

Sans authentification, `Cache-Control: no-store`, limité à 60 requêtes par minute et par IP (429 + `Retry-After` au-delà). Les listes d'accueil (une recherche sans `q`) et les nombres d'entrées sont gardés en mémoire 30 secondes : un dépôt rendu privé peut y rester listé aussi longtemps, son contenu restant protégé par les contrôles d'accès.

- **`GET /api/public/catalogs`** — un catalogue par format pris en charge (`format`, `name`, `label`, `entry_count`).
- **`GET /api/public/search`** — recherche dans les paquets npm et images Docker des dépôts publics hosted. Paramètres : `q` (nom, description, mots-clés ou tags ; 100 caractères max), `format` (`npm` ou `docker`), `owner` (`personal:<utilisateur>` ou `organization:<slug>`), `sort` (`relevance`, `updated` ou `popular`), `page` (1 à 100), `per_page` (1 à 50, 20 par défaut). La recherche tolère les fautes de frappe sur les noms (à partir de 3 caractères ; `match_kind` vaut alors `fuzzy` et ces résultats viennent en dernier). Chaque résultat indique aussi `downloads_7d`, le nombre de téléchargements des 7 derniers jours. Chaque résultat indique le propriétaire, le dépôt source et l'URL d'installation (`registry_url` pour npm, `image_reference` pour Docker).
- **`GET /api/search`** — la même recherche, pour un utilisateur connecté, sur tout ce qu'il peut lire : les dépôts de son organisation (proxys compris : un proxy ne contribue que ce qu'il a déjà mis en cache), les dépôts publics et ceux sur lesquels il a un droit. Mêmes paramètres, même classement et même format de réponse ; chaque `repository` porte en plus son `id` et son `repo_type` (`hosted` ou `proxy`). Un super-admin cherche partout. Nécessite un jeton (401 sinon).
- **`GET /api/public/suggest`** — suggestions de noms pendant la frappe (`q` : 2 à 100 caractères). Au plus 8 résultats (`kind`, `name`, dépôt et propriétaire), les meilleurs d'abord : nom exact, préfixe, sous-chaîne, puis approximatif. Budget propre : 120 requêtes par minute et par IP.
- **`GET /api/public/owners/{kind}/{slug}`** — résumé public d'un propriétaire (`kind` : `personal` ou `organization`) : nom d'affichage et nombres de dépôts, de paquets et d'images. 404 quand il n'a aucun dépôt public, ce qui est aussi la réponse pour un propriétaire inexistant : la page ne permet pas de deviner qu'un compte existe.
- **`GET /api/repositories/by-org/{slug}/{repo_name}`** — un dépôt public d'une organisation (404 sinon), pendant de `by-owner` pour les projets personnels. Comme `by-owner` et `GET /api/repositories/{id}`, un appelant anonyme reçoit le dépôt sans `quota_bytes`, `retention_keep_last_n` ni `organization_id` ; un utilisateur connecté garde la forme complète.

## Comparaison avec les alternatives

ArtiFerris n'est pas la seule option pour héberger un registre npm et/ou
Docker. Voici où il se situe face à trois références du secteur — sur le
périmètre fonctionnel et le coût de licence, pas sur des chiffres de
performance : aucun benchmark comparatif n'a été mené entre ces quatre
outils, et il serait malhonnête d'en inventer.

| | **ArtiFerris** | Nexus Repository (Community Edition) | Harbor | JFrog Artifactory |
|---|---|---|---|---|
| npm | ✅ | ✅ | ❌ | Payant (Pro) uniquement |
| Docker / OCI | ✅ | ✅ | ✅ | Payant (Pro) uniquement |
| Autres formats (Maven, PyPI, NuGet, Cargo, Helm…) | ❌ *(feuille de route)* | ✅ 20+ formats | OCI uniquement (Helm, SBOM, OPA…) | ✅ 60+ formats *(Pro)* |
| MFA | **Obligatoire**, natif (TOTP/passkey) | Optionnel, SSO en Pro | Optionnel | Optionnel, SSO en Pro |
| LDAP/OIDC | ✅ natif | Pro | ❌ | Pro |
| SAML | ❌ *(feuille de route)* | Pro | ❌ | Pro |
| Scan de vulnérabilités intégré | ✅ Trivy, natif | Produit séparé (Sonatype Lifecycle) | ✅ Trivy, natif | Payant (Xray) |
| Multi-tenant / projets isolés | ✅ organisations par sous-domaine | ✅ | ✅ | ✅ |
| Auto-hébergement gratuit | ✅ | ✅ (Community Edition) | ✅ (Apache 2.0, projet CNCF) | Java uniquement — Docker/npm exigent la version payante |

**Coût annuel estimé, auto-hébergé, hors infrastructure et exploitation**
(chiffres sourcés, pas de licence publique pour la plupart de ces
produits — voir les notes) :

- **ArtiFerris** — gratuit, aucune licence.
- **Harbor** — gratuit, Apache 2.0, projet CNCF, aucune offre payante.
- **Nexus Repository Community Edition** — gratuit pour npm, Docker,
  Maven, PyPI et une quinzaine d'autres formats. La version Pro (SSO,
  haute disponibilité, réplication) n'a pas de tarif public ; des
  estimations tierces la situent autour de 120 $/utilisateur/an, ou
  50 000–150 000+ $/an packagée avec la plateforme Sonatype
  complète[^nexus-pricing].
- **JFrog Artifactory** — la version open-source (Apache 2.0) ne couvre
  que l'écosystème Java (Maven/Gradle/Ivy) : ni Docker ni npm. Pour les
  deux formats que ArtiFerris couvre nativement et gratuitement, il faut la
  version Pro X, dont le tarif self-hosted annoncé démarre à
  27 000 $/an pour un serveur[^jfrog-pricing], et grimpe largement
  au-delà en configuration entreprise.

[^nexus-pricing]: [Sonatype Nexus Repository Pricing Guide — CloudRepo](https://www.cloudrepo.io/articles/sonatype-nexus-repository-pricing-guide)
[^jfrog-pricing]: [JFrog Artifactory Pricing Guide — CloudRepo](https://www.cloudrepo.io/articles/jfrog-artifactory-pricing-guide)

**Configuration matérielle recommandée** (chiffres tirés de la
documentation officielle de chaque produit, pas d'un test comparatif) :

| | **ArtiFerris**[^artiferris-bench] | Nexus Repository (Community Edition) | Harbor | JFrog Artifactory (Pro X, self-hosted) |
|---|---|---|---|---|
| CPU minimum | 0,5 cœur | 2 cœurs (profil « Small »)[^nexus-sysreq] | 2 cœurs[^harbor-prereqs] | 4 cœurs, jusqu'à 20 clients actifs[^jfrog-sizing] |
| CPU recommandé | 1 cœur | 4 à 8 cœurs selon le profil[^nexus-sysreq] | 4 cœurs[^harbor-prereqs] | 6 à 8 cœurs, jusqu'à 200 clients actifs[^jfrog-sizing] |
| RAM minimum | 128 Mo | 8 Go[^nexus-sysreq] | 4 Go[^harbor-prereqs] | 6 Go, jusqu'à 20 clients actifs[^jfrog-sizing] |
| RAM recommandée | 256 Mo | 8 à 32 Go selon le profil[^nexus-sysreq] | 8 Go[^harbor-prereqs] | 12 à 18 Go, jusqu'à 200 clients actifs[^jfrog-sizing] |
| Disque | non mesuré | ≥ 4 Go libres en permanence (sinon bascule en lecture seule) ; 500 Go+ courants avec Docker/Maven[^nexus-sysreq] | 40 Go minimum, 160 Go recommandé[^harbor-prereqs] | non chiffré dans la doc générale ; SSD conseillé[^jfrog-sysreq] |
| Base de données | PostgreSQL, obligatoire | H2 embarqué en évaluation, PostgreSQL recommandé en production[^nexus-sysreq] | PostgreSQL embarqué dans le bundle d'installation | PostgreSQL externe, obligatoire en production[^jfrog-sysreq] |
| Runtime | Binaire Rust natif, sans JVM | JVM, Java 21 requis[^nexus-sysreq] | Go, plusieurs conteneurs, pas de JVM | JVM, JDK 21 embarqué[^jfrog-sysreq] |

[^artiferris-bench]: Mesuré, pas documenté : conteneur `artiferris-api` limité via
    `docker run --cpus`/`--memory` (cgroup v2), face à 15-20 clients
    simulés (npm install/publish + docker pull/push, majoritairement en
    lecture) pendant 2-3 minutes. RAM et CPU lus directement dans
    `/sys/fs/cgroup/memory.current` et `cpu.stat` du conteneur, pas
    estimés. « Minimum » = 0,5 cœur / 128 Mo : la charge passe sans
    échec applicatif, mais avec un throttling CPU marqué (~68 % du temps
    d'exécution throttlé) et la RAM au ras du plafond. « Recommandé » =
    1 cœur / 256 Mo : 2185 requêtes, 2 échecs, throttling résiduel
    (~3 % du temps), RAM avec marge (pic à 92 Mo). Mesuré sur une
    machine de développement (pas un serveur dédié), donc pas
    directement comparable à la méthodologie des trois autres éditeurs,
    qui documentent des profils de dimensionnement pour des déploiements
    de production sur du matériel dédié.
[^nexus-sysreq]: [Sonatype Nexus Repository System Requirements](https://help.sonatype.com/en/sonatype-nexus-repository-system-requirements.html)
[^harbor-prereqs]: [Harbor Installation Prerequisites](https://goharbor.io/docs/2.13.0/install-config/installation-prereqs/)
[^jfrog-sizing]: [JFrog Hardware Sizing Matrix](https://docs.jfrog.com/installation/docs/hardware-sizing-matrix)
[^jfrog-sysreq]: [JFrog General System Requirements](https://docs.jfrog.com/installation/docs/general-system-requirements)

### Passage à l'échelle

Toujours mesuré, pas documenté : le tableau ci-dessus vient d'une charge
modeste (15-20 clients). Pour voir comment ArtiFerris encaisse davantage de
concurrence, même conteneur (4 cœurs / 2 Go), mais cette fois piloté par
un générateur de charge HTTP asynchrone (Python/aiohttp) plutôt que de
vrais processus CLI npm/docker par client — ça permet de monter à 100 et
200 clients simultanés sans multiplier les processus lourds côté machine
de test[^artiferris-scale] :

| Clients simultanés | Débit | Échecs | p95 (npm install) | CPU moyen | RAM (pic) |
|---|---|---|---|---|---|
| 20 | ~490 req/s | 0 | 90 ms | 108 % (de 4 cœurs) | 549 Mo |
| 100 | ~476 req/s | 0 | 360 ms | 108 % | 568 Mo |
| 200 | ~268 req/s | 0 | 1 781 ms | 81 % | 527 Mo |

Zéro échec applicatif à chaque palier, y compris à 200 clients : ArtiFerris
ralentit sous forte charge mais ne casse pas. Point moins flatteur, dit
tel quel : le débit **baisse** entre 100 et 200 clients (476 → 268 req/s)
alors que le CPU utilisé baisse aussi (108 % → 81 %) — signe d'un goulot
d'étranglement qui n'est pas le manque de cœurs bruts (pool de connexions
PostgreSQL ou contention sur l'event-loop async, sans doute, mais non
investigué). Un seul run par palier, sur une machine de développement :
à prendre comme un ordre de grandeur, pas une garantie de capacité.

**Plus de CPU, plus de débit** — à 100 clients toujours, doubler
l'allocation CPU fait clairement bouger le débit soutenu :

| Config | Débit à 100 clients | Throttling CPU |
|---|---|---|
| 4 cœurs / 2 Go | ~476 req/s | marqué |
| 8 cœurs / 2 Go | ~690 req/s | léger |

Pas de ligne « RAM recommandée pour 100 clients » ici, volontairement :
avec un client qui tape sans aucune limite de débit, la RAM observée
grimpe avec la **durée du test** (backlog de requêtes en attente qui
s'accumule), pas avec une consommation stable par client — sur 15 s elle
plafonnait à 1,2 Go, sur 60 s elle a rempli les 4 Go alloués. Un chiffre
RAM fiable demanderait un client de charge avec un débit plafonné
(requêtes/seconde réaliste plutôt que « à fond »), ce qui n'a pas été
fait.

[^artiferris-scale]: Générateur de charge : `aiohttp` en Python, appels HTTP
    directs sur les mêmes endpoints qu'un vrai client (métadonnées +
    tarball npm, jeton + manifeste + blob Docker), sans passer par les
    CLI `npm`/`docker`. Mix identique à la note précédente (majoritairement
    lecture). CPU/RAM lus dans les mêmes compteurs cgroup que le tableau
    ci-dessus.

**Ce que ces deux tableaux ne disent pas** : aucune mesure comparative
n'a été faite face à Nexus, Harbor ou Artifactory — les chiffres ci-dessus
ne concernent que ArtiFerris. Nexus et Harbor sont par ailleurs des projets
matures, déployés à grande échelle depuis des années, avec des
fonctionnalités que ArtiFerris n'a pas encore (voir la
[feuille de route](#feuille-de-route)) : SAML, haute disponibilité,
davantage de formats de paquets.

## Feuille de route

ArtiFerris est un projet actif, pas un produit figé : quotas, rétention, scan
de sécurité intégré (Trivy + npm audit), marque personnalisable et MFA
obligatoire sont déjà natifs, et la liste ci-dessous est celle des chantiers
qu'on a vraiment envie de mener ensuite — par ordre de priorité
approximatif.

**Solidifier les fondations**
- [ ] SAML — LDAP/Active Directory et OIDC sont déjà supportés, SAML pas
      encore
- [ ] Davantage de formats de paquets : Maven/Gradle, PyPI, NuGet, Cargo,
      Go modules, Helm charts, dépôts génériques/raw — `artiferris-npm`/
      `artiferris-docker` montrent déjà le patron d'adaptateur à suivre
- [ ] Backend de stockage objet (compatible S3) derrière
      `StorageBackendPort`, pour débloquer les déploiements multi-réplicas
      (aujourd'hui : un seul volume fichier, une seule réplique)
- [ ] Haute disponibilité / clustering, réplication géographique
- [ ] Signature/provenance des paquets (Sigstore, npm provenance)

**Voir plus grand : un registre ouvert**
- [ ] Espaces de noms par utilisateur au sein d'une organisation (scopes
      façon npm `@user/...`), distincts du modèle actuel où les dépôts
      appartiennent à l'organisation
- [ ] Accès en lecture public, non authentifié, pour les paquets publics
- [ ] Limitation de débit et prévention des abus pour le trafic anonyme
- [x] Pages de recherche et de découverte de paquets publiques
- [ ] Distribution d'artefacts mondiale via CDN

## Licence

Aucun fichier de licence n'est actuellement inclus dans ce dépôt —
considérez le code source comme tous droits réservés jusqu'à l'ajout
d'une licence.
