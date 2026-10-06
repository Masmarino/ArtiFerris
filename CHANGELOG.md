# Changelog

Toutes les évolutions notables d'ArtiFerris. Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/) et les
versions suivent [SemVer](https://semver.org/lang/fr/). Chaque section sert de texte à la release GitHub du même numéro : voir
« Publier une version » plus bas.

## [0.6.1] - 2026-10-06

### Ajouté

- **Recherche rapide**, comme dans FerrisGit : ⌘K (Ctrl K sous Windows et Linux), `/` ou le bouton de la barre, centré
  sous la palette qu'il ouvre (Gabarit 2.2.1). Avant toute saisie, elle propose les dépôts ouverts récemment dans ce
  navigateur, les pages du menu et les actions (« Nouveau dépôt », « Nouveau projet », « Déconnexion ») ; la saisie les
  filtre aussitôt, sans tenir compte des accents, puis ajoute les dépôts, les utilisateurs et les paquets ou images
  trouvés. Ses pages sont celles du menu : l'administration n'apparaît qu'à qui le menu la montre (rien pour un membre,
  Utilisateurs et Réglages pour un administrateur d'organisation), « Inviter un utilisateur » qu'au super-administrateur.
- **Réinitialisation du mot de passe par un administrateur** (`POST /api/users/{id}/reset-password`) : le mot de passe
  est annulé, toutes les sessions du compte sont fermées et un lien valable une heure est envoyé par e-mail (page
  `/reset-password`) ; si l'e-mail ne peut pas partir, le lien revient à l'administrateur pour qu'il le transmette.
  Refusée pour son propre compte, pour un compte encore invité et dans une organisation dont le fournisseur d'identité
  (LDAP, OIDC) gère les mots de passe. Migration `0015_password_resets`.
- **Réinitialisation de la double authentification par un administrateur** (`DELETE /api/users/{id}/mfa`), pour qui a
  perdu tous ses facteurs : application, codes de secours et clés d'accès sont retirés, toutes les sessions fermées ; la
  personne en configure un nouveau à sa prochaine connexion.
- **Documentation** sous `/docs`, en français : démarrer, utilisation (dépôts, npm, Docker, compte et jetons,
  catalogue public), administration (installation, configuration, organisations et utilisateurs, mises à jour,
  référencement) et API publique. Le lecteur vient de Gabarit (`@masmarino/gabarit/docs`) : recherche, sommaire, plan de
  la page. Sans session, elle s'affiche sous la barre des pages publiques ; avec une session, dans l'application (menu
  du compte). Elle se charge à la demande : `marked` et DOMPurify restent hors du bundle initial. Le serveur répond
  `404` à une page `.md` absente plutôt que l'application.
- **Limite des lectures anonymes npm et Docker** (#38) : par client (IPv4, /64 IPv6) et par minute, 1 200 par défaut
  (`ANONYMOUS_REGISTRY_READS_PER_MINUTE`, `0` pour supprimer la limite ; réponse `429` avec `Retry-After`, au format du
  registre pour Docker). Une requête avec un jeton n'est jamais comptée.
- **Compteurs partagés entre réplicas** pour tout le trafic anonyme (pages et API publiques, registres) : synchronisés
  par Postgres toutes les deux secondes, sans adresse en base (hachage à clé). Si la base ne répond pas, chaque réplica
  limite seul. Migration `0013_rate_limit_counters` (table non journalisée).

### Modifié

- **Pages alignées sur celles de FerrisGit** : compte, liste et fiche des utilisateurs, invitation (le lien d'activation
  s'affiche quand l'e-mail n'a pas pu partir), tableau de bord, santé et page introuvable reprennent sa structure et ses
  libellés. Le menu « Administration » regroupe, pour un super-administrateur, Tableau de bord, Utilisateurs,
  Organisations, Santé et Export ; pour un administrateur d'organisation, Utilisateurs puis Réglages de son organisation.
- **Le design de la famille Ferris vient désormais de Gabarit 2.0** : palette, IBM Plex (servie depuis le paquet), cadre
  graphite du shell, page de connexion. Le thème local (`styles/_theme.scss`), sa correspondance avec les jetons de
  Gabarit et les retouches du shell disparaissent ; seules restent les règles propres à ArtiFerris (barre publique,
  fond animé de connexion, panneau clair). Quelques teintes changent à peine : chaque texte atteint 7:1 sur la page et
  sur un panneau, d'où un texte secondaire plus clair en thème sombre et un bouton de danger un peu plus foncé.
- Les deux champs de recherche des pages publiques (barre du haut et catalogue) sont ceux de FerrisGit : le champ de
  Gabarit avec sa loupe, sans bouton « Rechercher » (Entrée lance la recherche, la saisie aussi après un court délai).
  Les suggestions restent annoncées comme une liste (combobox ARIA).
- La limite des pages et de l'API publiques passe d'une fenêtre glissante par horodatage à une fenêtre glissante par
  compteurs : même débit moyen, sans conserver chaque requête en mémoire. Les connexions échouées gardent leur limite par
  processus.
- **Nouvelle apparence**, commune à la famille Ferris (celle de FerrisGit et de son site) : IBM Plex, actions à l'encre,
  filets, coins de 2 à 6 px ; un cadre graphite autour des pages, avec la version claire du logo ; le chemin de la page
  avant son titre dans la barre (« Administration / Export ») ; les pages de connexion, d'inscription et d'activation sur
  un graphe de commits animé, qui s'arrête si le système demande moins de mouvement ; les pages publiques (catalogue,
  dépôt, paquet) sous la même barre graphite. Gabarit 1.3.0.
- **Pages de connexion, d'inscription et d'activation reprises du kit d'authentification de Gabarit** (1.4.0), comme
  FerrisGit : chaque erreur sous son champ ou dans une alerte, le focus là où il faut agir, la double authentification
  en étapes (clé d'accès recommandée, ou application et ses codes de secours), dans les cinq langues. L'invité choisit
  toujours son nom d'utilisateur à l'activation ; la connexion LDAP et OIDC, les avis (sessions fermées, lien SSO
  invalide) et le lien vers les paquets publics sont conservés.
- **Paramètres de sécurité du compte repris de Gabarit** : application d'authentification, codes de secours et clés
  d'accès, avec la date de dernière utilisation de chaque clé (migration `0014_passkey_last_used`). Une modification qui
  ferme les sessions renvoie à la connexion.
- **Codes de secours comme dans FerrisGit** : ils accompagnent aussi une clé d'accès configurée seule à la première
  connexion, se régénèrent avec n'importe quel facteur et ne disparaissent qu'avec le dernier (supprimer l'application
  les garde tant qu'une clé d'accès reste).
- Le lien d'activation envoyé par e-mail porte le jeton dans son fragment (`/activate#token=…`) : aucun serveur ni
  journal d'accès ne le voit, et la page le retire de la barre d'adresse. Les liens `?token=` déjà envoyés restent
  valables.

### Corrigé

- Accessibilité (WCAG 2.2 AA, vérifiée avec axe-core en clair, en sombre et sur téléphone) : le texte des statuts et le
  bord des champs atteignent les contrastes requis ; les commandes, les blocs de code et les tableaux qui défilent de
  côté (README, instructions d'utilisation, jeton créé) sont atteignables au clavier.
- Le logo de la barre de navigation repliée s'affiche : `Logo.png` était une icône dont le navigateur retenait une image
  vide.

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

[0.6.1]: https://github.com/Masmarino/ArtiFerris/releases/tag/v0.6.1
[0.6.0]: https://github.com/Masmarino/ArtiFerris/releases/tag/v0.6.0
