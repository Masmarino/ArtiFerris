# Organisations et utilisateurs

## Organisations

Chaque organisation vit sur son sous-domaine (`acme.artiferris.example`) avec ses dépôts, ses utilisateurs et sa
marque. Une organisation publique, créée d'office, sert les déploiements qui n'en ont qu'une.

- Un **administrateur d'organisation** n'agit que sur la sienne.
- Un **super-administrateur** agit sur toutes, depuis **Administration**, **Organisations**.

## Inviter des utilisateurs

Dans **Utilisateurs**, invitez quelqu'un par son adresse e-mail. Il reçoit un lien valable 24 heures et choisit
lui-même son nom d'utilisateur et son mot de passe ; l'administrateur ne saisit que l'adresse. Si l'e-mail ne part
pas, renvoyez l'invitation depuis la liste.

L'inscription libre, si elle est ouverte, ne vaut que pour l'organisation publique.

## Connexion par LDAP ou OIDC

Une organisation peut confier la connexion à un annuaire :

- **LDAP ou Active Directory** : les utilisateurs se connectent avec leur compte d'annuaire, sur le formulaire habituel.
- **OIDC** : la page de connexion envoie vers le fournisseur d'identité.

SAML n'est pas pris en charge. La double authentification reste demandée pour les comptes locaux.

## E-mails

Les invitations, les codes de connexion et les alertes de sécurité partent par le serveur SMTP réglé dans
**Administration**. Un e-mail de test vérifie la configuration. Chaque e-mail part dans la langue de son destinataire.

## Marque

Le logo et le favicon d'une organisation se changent dans **Administration**. Les fichiers sont validés par leur
contenu, pas par leur type déclaré.

## Export et import

Une instance avec une seule organisation peut exporter sa configuration et l'importer ailleurs. L'import se fait en
une transaction : ce qu'il ne peut pas restaurer est listé et ignoré, le reste est écrit d'un bloc.
