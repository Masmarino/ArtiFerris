import type { SettingsLabels } from './kit-labels'

const TOO_MANY_ATTEMPTS = 'Trop de tentatives, réessayez dans quelques minutes'
const PASSKEYS_UNAVAILABLE = "Les clés d'accès ne sont pas disponibles sur ce serveur."
const SIGNED_OUT = 'Vous avez été déconnecté. Reconnectez-vous pour continuer.'
const LAST_FACTOR =
  "C'est votre dernier facteur : vous devrez en configurer un nouveau à la prochaine connexion."
const on = (absolute: boolean) => (absolute ? ' le' : '')

export const FR_SETTINGS_LABELS = {
  mfaSettings: {
    heading: "Application d'authentification",
    enrollingHeading: "Configurer l'application d'authentification",
    codesHeading: 'Codes de secours',
    help: 'Google Authenticator, Authy, 1Password… : toute application TOTP convient.',
    codesHelp: "Conservez-les : ils ne s'affichent qu'une seule fois.",
    loading: 'Chargement de la double authentification…',
    loadFailed: "La double authentification n'a pas pu être chargée.",
    retry: 'Réessayer',
    enrollLead: 'Confirmez votre mot de passe pour commencer la configuration.',
    enrollSubmit: 'Continuer',
    enrollBusy: 'Préparation en cours',
    enrollFailed: "La configuration n'a pas pu démarrer, réessayez.",
    regenerateLead:
      'Confirmez votre mot de passe pour générer 10 nouveaux codes. Les anciens cesseront de fonctionner dès maintenant.',
    regenerateSubmit: 'Régénérer',
    regenerateBusy: 'Régénération en cours',
    regenerateFailed: "Les codes de secours n'ont pas pu être régénérés, réessayez.",
    disableSubmit: 'Continuer',
    disableCheckBusy: 'Vérification en cours',
    disableFailed: "La double authentification n'a pas pu être réinitialisée, réessayez.",
    removeTitle: "Supprimer l'application d'authentification",
    removeHelp:
      "Supprime votre application actuelle. Vous serez déconnecté de tous vos appareils et vous vous reconnecterez avec votre clé d'accès.",
    removeAction: 'Supprimer',
    removeDialogHeading: "Supprimer l'application d'authentification ?",
    removeDialogMessage:
      "Vous serez déconnecté de tous vos appareils. Vous vous reconnecterez avec votre clé d'accès, sans application.",
    removePromptLead:
      "Confirmez votre mot de passe pour supprimer votre application d'authentification.",
    removeBusy: 'Suppression en cours',
    resetTitle: 'Réinitialiser la double authentification',
    resetHelp:
      'Retire votre application actuelle. Vous serez déconnecté de tous vos appareils et devrez en configurer une nouvelle à votre prochaine connexion.',
    resetAction: 'Réinitialiser',
    resetDialogHeading: 'Réinitialiser la double authentification ?',
    resetDialogMessage:
      "Vous serez déconnecté et devrez configurer une nouvelle application d'authentification à votre prochaine connexion.",
    resetPromptLead: 'Confirmez votre mot de passe pour réinitialiser la double authentification.',
    resetBusy: 'Réinitialisation en cours',
    codesLeft: (count) =>
      count === 0
        ? 'Aucun code de secours restant'
        : count === 1
          ? '1 code de secours restant'
          : `${count} codes de secours restants`,
    lowCodes: "Régénérez-en pour ne pas perdre l'accès à votre compte.",
    scanLead:
      "Scannez ce QR code avec votre application d'authentification, puis saisissez le code à 6 chiffres qu'elle affiche.",
    codeLabel: 'Code à 6 chiffres',
    activate: 'Activer',
    verifying: 'Vérification en cours',
    cancel: 'Annuler',
    close: 'Fermer',
    codesLeadEnrolled:
      "La double authentification est activée. Ces codes ne s'afficheront plus : ils vous permettent de vous connecter si vous perdez l'accès à votre application, et chacun ne fonctionne qu'une seule fois.",
    codesLeadRegenerated:
      "Voici vos nouveaux codes de secours. Les anciens codes ne fonctionnent plus. Ces codes ne s'afficheront plus, et chacun ne fonctionne qu'une seule fois.",
    dontLeave: 'Ne quittez pas cette page avant de les avoir enregistrés.',
    done: 'Terminé',
    appConfigured: 'Application configurée',
    enabled: 'Activée',
    appHelpWithPasskey:
      "Un code à 6 chiffres, généré par votre application, peut remplacer votre clé d'accès après votre mot de passe.",
    appHelp:
      'Un code à 6 chiffres, généré par votre application, est demandé après votre mot de passe.',
    noApp: 'Aucune application configurée',
    optional: 'Facultative',
    noAppHelp:
      "Votre clé d'accès suffit pour vous connecter. Ajoutez une application pour disposer d'un second moyen : elle génère un code à 6 chiffres.",
    addApp: 'Ajouter une application',
    backupCodesTitle: 'Codes de secours',
    backupCodesHelp:
      "Chaque code ne fonctionne qu'une fois. En générer de nouveaux invalide les précédents.",
    regenerateCodes: 'Régénérer les codes de secours',
    noFactorWarning:
      'La double authentification est obligatoire : vous devrez la configurer à votre prochaine connexion.',
    setUpNow: 'Configurer maintenant',
    currentPassword: 'Mot de passe actuel',
    showPassword: 'Afficher le mot de passe',
    hidePassword: 'Masquer le mot de passe',
    enterPassword: 'Saisissez votre mot de passe',
    wrongPassword: 'Mot de passe incorrect',
    enterCode: 'Saisissez le code à 6 chiffres de votre application',
    wrongCode: 'Code incorrect',
    activationFailed: "L'activation a échoué, réessayez.",
    tooManyAttempts: TOO_MANY_ATTEMPTS,
    signedOut: SIGNED_OUT,
  },
  passkeySettings: {
    heading: "Clés d'accès",
    description:
      "Validez votre connexion avec l'empreinte digitale, le visage ou le code de votre appareil, ou avec une clé de sécurité.",
    loading: "Chargement des clés d'accès…",
    loadFailed: "Les clés d'accès n'ont pas pu être chargées.",
    retry: 'Réessayer',
    browserBlocked:
      "Ce navigateur ne prend pas en charge les clés d'accès : vous ne pouvez pas en ajouter ici, mais vous pouvez supprimer celles que vous avez déjà.",
    serverBlocked: `${PASSKEYS_UNAVAILABLE} Vous ne pouvez pas en ajouter, mais vous pouvez supprimer celles que vous avez déjà.`,
    emptyHeading: "Aucune clé d'accès",
    emptyMessage: "Ajoutez-en une pour valider votre connexion d'un geste, sans code à recopier.",
    listLabel: "Clés d'accès enregistrées",
    added: (absolute) => `Ajoutée${on(absolute)}`,
    lastUsed: (absolute) => `Dernière utilisation${on(absolute)}`,
    neverUsed: 'Jamais utilisée',
    delete: 'Supprimer',
    deleteKey: (name) => `Supprimer la clé ${name}`,
    passkeyAdded: (name) => `Clé d'accès « ${name} » ajoutée.`,
    passkeyGone: (name) => `La clé d'accès « ${name} » n'existe plus.`,
    addPasskey: "Ajouter une clé d'accès",
    finishAppFirst: "Terminez d'abord la configuration de l'application d'authentification.",
    deleteDialogHeading: "Supprimer cette clé d'accès ?",
    deleteDialogMessage: (name, lastFactor) =>
      `« ${name} » ne permettra plus de vous connecter. Vous serez déconnecté de tous vos appareils et devrez vous reconnecter.` +
      (lastFactor ? ` ${LAST_FACTOR}` : ''),
    deleteConfirm: 'Supprimer',
    cancel: 'Annuler',
    close: 'Fermer',
    deleting: 'Suppression en cours',
    addLead:
      'Nommez cette clé et confirmez votre mot de passe : votre appareil vous demandera ensuite de valider.',
    promptOpen: 'Validez sur votre appareil pour créer la clé.',
    nameLabel: 'Nom de la clé (facultatif)',
    defaultPasskeyName: "Clé d'accès",
    currentPassword: 'Mot de passe actuel',
    showPassword: 'Afficher le mot de passe',
    hidePassword: 'Masquer le mot de passe',
    create: "Créer la clé d'accès",
    creating: 'Création en cours',
    deleteLead: (name) => `Confirmez votre mot de passe pour supprimer « ${name} ».`,
    lastFactorWarning: LAST_FACTOR,
    continue: 'Continuer',
    checking: 'Vérification en cours',
    nameTooLong: (max) => `Le nom ne doit pas dépasser ${max} caractères`,
    nameControlCharacters: 'Le nom ne peut pas contenir de caractères de contrôle',
    enterPassword: 'Saisissez votre mot de passe',
    wrongPassword: 'Mot de passe incorrect',
    tooManyPasskeys:
      "Vous avez atteint le nombre maximal de clés d'accès : supprimez-en une avant d'en ajouter.",
    alreadyRegistered: 'Cette clé est déjà enregistrée',
    passkeysUnavailable: PASSKEYS_UNAVAILABLE,
    addFailed: "La clé d'accès n'a pas pu être créée, réessayez.",
    cancelled: 'Opération annulée',
    browserUnsupported: "Ce navigateur ne prend pas en charge les clés d'accès.",
    deleteFailed: "La clé d'accès n'a pas pu être supprimée, réessayez.",
    tooManyAttempts: TOO_MANY_ATTEMPTS,
    signedOut: SIGNED_OUT,
  },
} satisfies SettingsLabels
