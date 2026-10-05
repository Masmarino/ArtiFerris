# API publique

Ces routes servent le catalogue public. Elles ne demandent pas de compte, sont limitées à 60 requêtes par minute et
par adresse IP (`429` avec `Retry-After` au-delà) et répondent avec `Cache-Control: no-store`.

## Catalogue

### `GET /api/public/catalogs`

Un catalogue par format : `format`, `name`, `label`, `entry_count`.

### `GET /api/public/search`

| Paramètre | Rôle |
| --- | --- |
| `q` | Le texte cherché, 100 caractères au plus. Tolère les fautes de frappe dès 3 caractères. |
| `format` | `npm` ou `docker`. |
| `owner` | `personal:<utilisateur>` ou `organization:<organisation>`. |
| `sort` | `relevance`, `updated` ou `popular`. |
| `page`, `per_page` | La page (1 à 100) et sa taille (1 à 50, 20 par défaut). |

Chaque résultat porte le propriétaire, le dépôt, l'adresse d'installation et `downloads_7d`.

### `GET /api/public/suggest`

Les suggestions de la recherche : `q` de 2 à 100 caractères, 8 résultats au plus. Limite propre : 120 requêtes par
minute et par adresse IP.

### `GET /api/public/owners/{kind}/{slug}`

Le nom d'un propriétaire et le nombre de ses dépôts, paquets et images publics. `404` s'il n'a rien de public.

## Paquets et images

### `GET /api/repositories/{id}/packages/npm/{name}`

Les versions, les dist-tags, `downloads_7d`, le README nettoyé (`readme_html`, `null` sans README) et l'adresse du
registre.

### `GET /api/repositories/{id}/packages/docker/{image}`

Les étiquettes avec leur taille (`null` pour un index multi-architecture), `downloads_7d` et la référence de l'image.

> **Note** : les téléchargements comptent les récupérations d'archives npm et de manifests Docker sur les dépôts
> hébergés, par jour, sans donnée sur l'utilisateur, et sont purgés après 13 mois.
