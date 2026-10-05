# Référencement

Les pages du catalogue public ont leur propre en-tête pour les moteurs de recherche et les aperçus de liens : titre,
description, URL canonique, Open Graph et données structurées pour les paquets.

## Activer l'indexation

**L'indexation est désactivée par défaut.** Un super-administrateur l'active dans **Administration**, dans les
paramètres de l'organisation publique.

- Désactivée : toutes les pages demandent à ne pas être indexées, `robots.txt` interdit tout et le plan du site est
  vide.
- Activée : `robots.txt` interdit l'API, les registres et l'application, et `/sitemap.xml` liste les pages publiques.
  Les pages de recherche restent hors index.

## Par organisation

Dans les paramètres de chaque organisation :

- **Page publique** : fermée, les pages de l'organisation répondent comme si rien n'était publié.
- **Bloquer les moteurs de recherche** : les pages restent visibles mais demandent à ne pas être indexées.

Les deux sont ouverts par défaut ; l'interrupteur de l'instance reste prioritaire.

## Langue

Les pages répondent dans la langue de l'en-tête `Accept-Language` du visiteur, en anglais sinon (donc pour un robot).
L'URL est la même dans toutes les langues.
