# Installation

ArtiFerris tient dans une image : le serveur, les deux registres, l'interface et Trivy. Il lui faut une base
PostgreSQL et un volume pour les paquets et les images.

## Avec Docker Compose

```sh
cp .env.example .env
docker compose up -d --build
```

Renseignez d'abord dans `.env` : `POSTGRES_PASSWORD`, `JWT_SECRET`, `SECRETS_ENCRYPTION_KEY`, `PUBLIC_URL` et le
premier administrateur (`ARTIFERRIS_BOOTSTRAP_ADMIN_USERNAME`, `ARTIFERRIS_BOOTSTRAP_ADMIN_PASSWORD`). Générez les
secrets ainsi :

```sh
openssl rand -base64 48   # JWT_SECRET, SECRETS_ENCRYPTION_KEY
openssl rand -hex 24      # POSTGRES_PASSWORD, qui finit dans une URL
```

Le serveur refuse de démarrer si un secret fait moins de 32 octets ou commence par `change-me`, si le mot de passe du
premier administrateur est encore celui de l'exemple, ou si `PUBLIC_URL` pointe sur `0.0.0.0`.

## Avec Helm

```sh
helm upgrade --install artiferris ./helm/artiferris \
  --namespace artiferris --create-namespace \
  --set image.tag=<version> \
  --set ingress.host=app.example.com --set ingress.wildcardHost='*.example.com' \
  --set artiferris.baseDomain=example.com
```

- `image.tag` n'a pas de valeur par défaut ; `image.digest` épingle l'image.
- Le chart génère le mot de passe de la base, `JWT_SECRET` et `SECRETS_ENCRYPTION_KEY` et les retrouve d'un déploiement
  à l'autre.

> **Attention** : avec `helm template`, `--dry-run`, Argo CD ou Flux, le chart ne retrouve pas les secrets qu'il a
> générés. Fixez `secrets.postgresPassword`, `secrets.jwtSecret` et `secrets.secretsEncryptionKey`, sinon chaque rendu
> en invente de nouveaux et les secrets stockés deviennent illisibles.

- Le Secret et les deux volumes (registre et base) survivent à `helm uninstall`.
- Avec l'ingress, renseignez `artiferris.trustedProxyIps` (le réseau des pods du contrôleur) : sans cela tous les
  visiteurs partageraient un seul compteur de connexions, et le chart refuse de se rendre.

## Vérifier

- `/healthz` répond tant que le processus tourne.
- `/readyz` vérifie en plus que la base répond.

Pour une quinzaine de clients, 1 cœur et 256 Mo suffisent.

La suite : la [configuration](/docs/administration/configuration).
