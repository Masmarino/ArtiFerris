# Mises à jour

## Mettre à jour

1. Sauvegardez la base.
2. Déployez la nouvelle version.
3. Vérifiez la connexion (SSO compris), les e-mails et la double authentification.

Les migrations de la base s'appliquent au démarrage.

## Revenir en arrière

> **Attention** : revenir à une version précédente demande de restaurer la base. `helm rollback` et `--atomic`
> n'annulent pas les migrations déjà appliquées, et une version plus ancienne peut refuser de démarrer sur un schéma
> plus récent.

## Changer la clé de chiffrement

La clé `SECRETS_ENCRYPTION_KEY` chiffre les secrets stockés : identifiants SMTP, LDAP, OIDC et des proxys, graines
TOTP.

1. Sauvegardez la base.
2. Mettez la nouvelle clé dans `SECRETS_ENCRYPTION_KEY`, l'ancienne dans `SECRETS_ENCRYPTION_KEY_PREVIOUS`, et
   `SECRETS_REENCRYPT_LEGACY=true`.
3. Redémarrez tous les réplicas ensemble : un réplica resté sur l'ancienne clé ne lit pas ce qu'un autre a réécrit.
4. La ligne `re-encrypted stored secrets` du journal confirme la conversion.
5. Retirez `SECRETS_ENCRYPTION_KEY_PREVIOUS`.

Avec Helm : `secrets.secretsEncryptionKey`, `secrets.secretsEncryptionKeyPrevious` et
`artiferris.reencryptStoredSecrets`.

Une valeur illisible (mauvaise clé, base restaurée) est signalée dans le journal ; ce qui en dépend reste en panne
jusqu'à la bonne clé ou une nouvelle saisie.
