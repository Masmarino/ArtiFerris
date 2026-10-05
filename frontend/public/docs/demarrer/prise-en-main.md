# Prise en main

Cette page vous mène de l'invitation à votre premier paquet publié.

## Activer son compte

Un administrateur vous invite par e-mail. Le lien d'activation vaut 24 heures et ne sert qu'une fois : choisissez-y
votre nom d'utilisateur et votre mot de passe (8 caractères au moins).

> **Note** : si l'instance accepte les inscriptions libres, le lien « Créer un compte » de la page de connexion vous y
> mène directement.

## Protéger sa connexion

La double authentification est obligatoire. À votre première connexion, choisissez :

- **une clé d'accès** (recommandé) : Touch ID, Windows Hello, un code PIN ou une clé de sécurité ; rien à saisir ;
- **une application d'authentification** : Google Authenticator, Authy, 1Password… ; un code à 6 chiffres à chaque
  connexion.

Dans les deux cas, ArtiFerris vous remet ensuite dix **codes de secours**. Chacun ne sert qu'une fois : gardez-les en
lieu sûr, ils vous permettent de vous connecter si vous perdez votre appareil.

## Créer un jeton API

Les clients npm et Docker ne passent pas par la double authentification : ils s'identifient avec un jeton. Dans
**Mon compte**, onglet **Jetons API**, créez-en un et copiez-le : il ne s'affiche qu'une fois.

## Publier un premier paquet

1. Ouvrez **Dépôts** et choisissez un dépôt npm hébergé sur lequel vous avez le droit d'écriture, ou créez-en un.
2. Sa page **Utilisation** donne les lignes à ajouter à votre `.npmrc`, avec l'adresse exacte du dépôt.
3. Publiez :

```sh
npm publish
```

Le paquet apparaît dans le dépôt, avec son README et la commande d'installation. Pour Docker, voir
[Registre Docker](/docs/utilisation/docker).
