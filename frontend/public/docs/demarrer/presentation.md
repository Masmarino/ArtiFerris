# Présentation

ArtiFerris est un gestionnaire d'artefacts auto-hébergé. Il réunit un registre npm et un registre Docker/OCI derrière
une seule console, une seule authentification et les mêmes contrôles : droits, quotas, rétention et audit.

## Ce qu'il contient

- **Deux registres** : npm (publication, installation, dépublication, dist-tags) et Docker/OCI (push, pull,
  suppression de manifests).
- **Trois types de dépôt** pour chaque format : hébergé, proxy devant un registre amont, et groupe qui réunit plusieurs
  dépôts derrière une seule adresse. Voir [Dépôts](/docs/utilisation/depots).
- **La sécurité des artefacts** : audit des dépendances npm à chaque publication, scan des images Docker avec Trivy à
  chaque push.
- **Un catalogue public** que les visiteurs parcourent sans compte, si l'administrateur l'ouvre. Voir
  [Catalogue public](/docs/utilisation/catalogue-public).
- **Plusieurs organisations** sur une même instance, chacune sur son sous-domaine, avec ses dépôts, ses utilisateurs et
  sa marque.
- **Des comptes protégés** : double authentification obligatoire (application d'authentification ou clé d'accès),
  codes de secours, jetons API, connexion par LDAP ou OIDC.

L'interface est disponible en français, en anglais, en espagnol, en italien et en allemand. La langue de votre compte
prime sur celle du navigateur ; les e-mails suivent la langue du destinataire.

## Comment il est construit

Un seul binaire Rust sert l'API, les deux registres et l'interface. Les données vivent dans PostgreSQL, les paquets
et les couches d'images dans un répertoire de données.

## Pour commencer

- Vous avez reçu une invitation : suivez la [prise en main](/docs/demarrer/prise-en-main).
- Vous installez l'instance : commencez par l'[installation](/docs/administration/installation).
