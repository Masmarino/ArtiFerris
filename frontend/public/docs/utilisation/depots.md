# Dépôts

Un dépôt contient des paquets npm ou des images Docker, jamais les deux. Il appartient à une organisation ou à un
utilisateur.

## Les trois types

| Type | Rôle |
| --- | --- |
| **Hébergé** | Vos propres paquets ou images, publiés ici. |
| **Proxy** | Un cache devant un registre amont (par exemple `registry.npmjs.org`) : la première demande va chercher le paquet en amont, les suivantes le servent depuis ArtiFerris. |
| **Groupe** | Plusieurs dépôts du même format derrière une seule adresse. Le premier membre qui contient le paquet demandé répond, dans l'ordre que vous fixez. |

Un proxy peut s'identifier auprès du registre amont avec un nom d'utilisateur et un mot de passe, ou un jeton. Il
n'envoie ces identifiants qu'à l'adresse configurée, en `https` ; ailleurs, il tire anonymement.

## Adresse d'un dépôt

- Un dépôt d'organisation : `/npm/<dépôt>/` pour npm, `<hôte>/<dépôt>/<image>` pour Docker, sur le sous-domaine de
  l'organisation.
- Un dépôt personnel : `/npm/u/<utilisateur>/<dépôt>/` et `<hôte>/u/<utilisateur>/<dépôt>/<image>`.

La page **Utilisation** de chaque dépôt donne les commandes exactes.

## Droits

Chaque dépôt accorde des droits par utilisateur :

| Droit | Permet |
| --- | --- |
| `read` | Installer et tirer. |
| `write` | Publier et pousser, en plus de lire. |
| `admin` | Régler le dépôt et ses droits, en plus d'écrire. |

Lire à travers un groupe demande `read` sur chacun de ses membres. Les droits sont vérifiés par le serveur sur chaque
route, pas seulement masqués dans l'interface.

Un dépôt **public** se lit sans compte et apparaît dans le [catalogue public](/docs/utilisation/catalogue-public).

## Quota et rétention

- **Quota** : la place que le dépôt peut occuper. Une publication qui le dépasserait est refusée.
- **Rétention** : ne garder que les N dernières versions ou étiquettes. La purge passe toutes les 6 heures ;
  `latest` n'est jamais purgé.

> **Attention** : republier une version npm dépubliée est refusé, même après une purge. Publiez une nouvelle version.
