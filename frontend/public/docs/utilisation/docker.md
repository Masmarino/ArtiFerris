# Registre Docker

ArtiFerris est un registre Docker/OCI : `docker`, `podman` et les outils qui parlent ce protocole s'en servent
directement.

## Se connecter

Le mot de passe est un jeton API (voir [Compte et jetons](/docs/utilisation/compte-et-jetons)) :

```sh
echo <votre-jeton> | docker login acme.artiferris.example -u <votre-nom-utilisateur> --password-stdin
```

## Pousser et tirer

```sh
docker tag mon-image:1.0 acme.artiferris.example/images/mon-image:1.0
docker push acme.artiferris.example/images/mon-image:1.0
docker pull acme.artiferris.example/images/mon-image:1.0
```

Pour un dépôt personnel, l'image vit sous `u/<utilisateur>/<dépôt>/`. La page **Utilisation** du dépôt donne les
commandes exactes.

## Scan des images

Chaque push lance un scan [Trivy](https://github.com/aquasecurity/trivy) de l'image. Ses résultats s'affichent sur la
page de l'image ; vous pouvez relancer un scan à la main.

## Taille et nettoyage

La page d'une image donne la taille de chaque étiquette (sauf pour un index multi-architecture). Les envois abandonnés
sont nettoyés toutes les heures, et les couches que plus rien ne référence après 48 heures.

> **Note** : les jetons d'accès que Docker obtient après `docker login` durent 2 minutes ; le client les renouvelle
> seul.
