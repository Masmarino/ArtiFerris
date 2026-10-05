# Configuration

Le serveur se règle par des variables d'environnement. `docker-compose.yml` câble celles d'un déploiement sur un seul
nœud.

## Obligatoires

| Variable | Rôle |
| --- | --- |
| `DATABASE_URL` | La connexion PostgreSQL. |
| `JWT_SECRET` | Signe les sessions et les jetons Docker. 32 octets aléatoires au moins. Le changer ferme toutes les sessions et tous les `docker login`. |
| `SECRETS_ENCRYPTION_KEY` | Chiffre les secrets stockés (SMTP, LDAP, OIDC, identifiants des proxys, graines TOTP). Différente de `JWT_SECRET`, et aléatoire. |
| `ARTIFERRIS_BASE_DOMAIN` | Le domaine des organisations : `artiferris.example` pour `acme.artiferris.example`. |
| `PUBLIC_URL` | L'adresse externe : liens d'invitation, origine des clés d'accès, URL canoniques. Sauf en local. |
| `ARTIFERRIS_BOOTSTRAP_ADMIN_USERNAME`, `ARTIFERRIS_BOOTSTRAP_ADMIN_PASSWORD` | Le premier administrateur, créé seulement si aucun utilisateur n'existe. |

## Réseau et sécurité

| Variable | Défaut | Rôle |
| --- | --- | --- |
| `TRUSTED_PROXY_IPS` | | Les reverse proxies dont l'en-tête `X-Forwarded-For` est cru (plages CIDR). |
| `ARTIFERRIS_SSRF_ALLOWED_CIDRS` | | Les plages privées que LDAP, SMTP, OIDC et les proxys peuvent joindre. Sinon les adresses privées, loopback et link-local sont refusées. |
| `ANONYMOUS_REGISTRY_READS_PER_MINUTE` | `1200` | Lectures anonymes npm et Docker par minute et par client ; `0` supprime la limite. |
| `CORS_ALLOWED_ORIGIN` | | Restreint le CORS à une origine. À laisser vide quand l'interface est servie par le serveur. |
| `ARTIFERRIS_DOCKER_TOKEN_REALM` | | Le realm des jetons Docker, à fixer seulement derrière un intermédiaire qui ne transmet pas fidèlement l'hôte et le schéma. |

## Stockage et fonctionnement

| Variable | Défaut | Rôle |
| --- | --- | --- |
| `STORAGE_ROOT` | `./data` | Les archives npm et les couches Docker. Un volume persistant en production. |
| `BIND_ADDR` | `0.0.0.0:8080` | L'adresse d'écoute. |
| `DB_MAX_CONNECTIONS` | `10` | La taille du pool de connexions. |
| `AUDIT_RETENTION_DAYS` | `365` | La durée du journal d'audit ; `0` garde tout. |
| `RUST_LOG` | | Le filtre des journaux, par exemple `info`. |

## Rotation de la clé

`SECRETS_ENCRYPTION_KEY_PREVIOUS` et `SECRETS_REENCRYPT_LEGACY` servent à changer la clé de chiffrement : voir
[Mises à jour](/docs/administration/mises-a-jour).

> **Note** : certaines tâches ne se règlent pas : la purge de la rétention passe toutes les 6 heures, celle des
> métriques et des envois Docker abandonnés toutes les heures.
