# Registre npm

ArtiFerris parle le protocole du registre npm : `npm`, `pnpm` et `yarn` s'en servent comme de `registry.npmjs.org`.

## Configurer le client

Créez d'abord un jeton API (voir [Compte et jetons](/docs/utilisation/compte-et-jetons)), puis ajoutez à votre
`.npmrc` les lignes données par la page **Utilisation** du dépôt. Pour un dépôt d'organisation :

```ini
registry=https://acme.artiferris.example/npm/paquets/
//acme.artiferris.example/npm/paquets/:_authToken=<votre-jeton>
```

Pour lire seulement un dépôt public, la ligne `registry` suffit.

## Publier

```sh
npm publish
```

À chaque publication, ArtiFerris audite les dépendances du paquet. Un paquet a au plus 5 000 versions et 32 Mio de
manifestes.

## Installer

```sh
npm install mon-paquet
```

À travers un dépôt **groupe**, une seule adresse sert vos paquets hébergés et ceux d'un proxy devant
`registry.npmjs.org`.

## Étiquettes et dépublication

```sh
npm dist-tag add mon-paquet@2.1.0 beta
npm unpublish mon-paquet@2.0.0
```

> **Attention** : une version dépubliée ne peut pas être republiée. Publiez une nouvelle version.

## Auditer

`npm audit` fonctionne contre ArtiFerris. Les noms et versions des paquets audités partent vers `registry.npmjs.org` :
ne pointez pas `npm audit` sur ArtiFerris si ces noms ne doivent pas quitter votre réseau.
