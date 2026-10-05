# Compte et jetons

Tout se règle dans **Mon compte** : le profil et le mot de passe, la sécurité, les jetons API.

## Double authentification

Elle est obligatoire. Dans l'onglet **Sécurité** :

- **Application d'authentification** : ajoutez-en une, ou retirez-la si vous gardez une clé d'accès.
- **Clés d'accès** : ajoutez-en plusieurs (un ordinateur, un téléphone, une clé de sécurité), supprimez celles que vous
  n'utilisez plus. La date de dernière utilisation de chacune s'affiche.

Chaque modification qui demande votre mot de passe vous déconnecte de tous vos appareils.

## Codes de secours

Dix codes vous sont remis avec votre premier facteur. Chacun remplace une fois votre application ou votre clé d'accès.
Quand il en reste peu, régénérez-en dix : les anciens cessent aussitôt de fonctionner. Ils disparaissent avec votre
dernier facteur.

## Jetons API

Les clients npm et Docker, et vos scripts, s'identifient avec un jeton API. Dans l'onglet **Jetons API** :

1. Créez un jeton et donnez-lui un nom qui dit où il sert (« ordinateur portable », « CI »).
2. Copiez-le : il ne s'affiche qu'une fois.
3. Révoquez-le dès qu'il ne sert plus.

Un administrateur peut voir et révoquer les jetons de tous.

## Sessions

**Se déconnecter partout** ferme toutes vos sessions et révoque vos jetons Docker et API, y compris celle en cours.
C'est aussi la seule façon de couper l'accès d'un compte qui se connecte par SSO, sans mot de passe à changer.
