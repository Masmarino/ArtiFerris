# Changelog

Toutes les évolutions notables d'ArtiFerris. Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/) et les
versions suivent [SemVer](https://semver.org/lang/fr/). Chaque section sert de texte à la release GitHub du même numéro : voir
« Publier une version » plus bas.

## [0.6.0] - 2026-10-01

### Changements incompatibles de l'API

- `POST /api/users` et `POST /api/organizations/{id}/users` n'acceptent plus `username` : l'administrateur ne saisit que l'adresse
  e-mail.
- `POST /api/auth/activate` exige désormais `username` : l'invité choisit son nom d'utilisateur en activant son compte.
- L'événement d'audit `UserInvited` enregistre `email` à la place de `username`. Les anciens événements gardent leur contenu et
  restent lisibles.

### Ajouté

- **Interface en cinq langues** (français, anglais, espagnol, italien, allemand) : langue du navigateur au premier passage,
  changement à chaud, choix enregistré sur le compte (`PUT /api/me/language`, `language` dans `GET /api/me`).
- **E-mails dans la langue du destinataire** (création de compte, changement de mot de passe, second facteur ajouté). Une
  invitation est écrite dans la langue de l'administrateur qui l'envoie.
- **Pages publiques et SEO dans la langue du lecteur** : titre, description, Open Graph et données structurées suivent
  `Accept-Language` (anglais par défaut, pour les robots) ; l'URL canonique reste la même dans toutes les langues.
- **Codes d'erreur stables** (`code`) dans les réponses de l'API ; l'interface les traduit au lieu de lire le texte du serveur.
- **Page publique par organisation** : un administrateur peut fermer les pages publiques de son organisation ou demander aux
  moteurs de recherche de les ignorer. Sur l'organisation publique, la fermeture vaut pour toute l'instance (super-administrateur
  uniquement).
- **Choix du nom d'utilisateur à l'activation** (#62) : le nom est validé et son unicité vérifiée, sans tenir compte de la casse,
  avant que l'invitation soit consommée, puis une seconde fois par la base. Si le nom est refusé, le lien d'invitation reste
  utilisable.

### Corrigé

- Les scans Trivy des images Docker échouaient pour tout dépôt hors de l'organisation publique (#132).
- Un format inconnu dans l'URL d'un paquet et `/@/dépôt` affichent la page « introuvable » au lieu d'une page blanche ou d'une
  erreur (#88).
- La liste des jetons d'API signale un échec de chargement (#88).
- Plusieurs tests instables (limite de corps, liste de 500 jetons, routes de l'application) ne dépendent plus de la charge de la
  machine (#126), et un workflow `Flaky tests` répète les suites sous charge.

### Modifié

- `@angular-devkit/build-angular`, inutilisé, est retiré : `npm audit` ne signale plus rien (#88).
- Commentaires et documentation resserrés (README FR et EN, Helm, `.env.example`, `docker-compose.yml`) ; aucun comportement ne
  change.

### Migrations

Deux migrations s'appliquent au démarrage, sans action manuelle : `0011_user_preferences` (table des préférences d'interface) et
`0012_public_page_controls` (deux colonnes dans `system_settings`, pages ouvertes et indexables comme avant). Voir « Mise à jour »
dans le README avant de mettre à niveau.

### Comptes invités et import de configuration

Un compte invité garde un nom provisoire `invite-<hex>` jusqu'à son activation ; les listes d'administration affichent alors son
adresse. Les comptes restaurés par l'import de configuration gardent leur nom, mais la page d'activation le leur redemande.

## [0.5.0] et versions antérieures

Pas de journal des changements avant la 0.6.0 : voir les [tags](https://github.com/Masmarino/ArtiFerris/tags) et l'historique des
commits.

## Publier une version

1. Passer la version des six `Cargo.toml` (et `Cargo.lock`) au nouveau numéro.
2. Ajouter en haut de ce fichier une section `## [X.Y.Z] - AAAA-MM-JJ`, au même format que ci-dessus.
3. Fusionner sur `main`, puis pousser le tag `vX.Y.Z` sur ce commit.

Le workflow CI/CD construit l'image, la publie, puis crée la release GitHub `vX.Y.Z` avec le texte de la section correspondante.
Si la section manque, le job de release échoue (l'image et le déploiement ne sont pas touchés) : ajoutez-la, puis relancez le job.

[0.6.0]: https://github.com/Masmarino/ArtiFerris/releases/tag/v0.6.0
