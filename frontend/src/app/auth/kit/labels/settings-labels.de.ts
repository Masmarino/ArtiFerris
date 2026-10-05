import type { SettingsLabels } from './kit-labels'

const TOO_MANY_ATTEMPTS = 'Zu viele Versuche, versuchen Sie es in einigen Minuten erneut'
const PASSKEYS_UNAVAILABLE = 'Passkeys sind auf diesem Server nicht verfügbar.'
const SIGNED_OUT = 'Sie wurden abgemeldet. Melden Sie sich erneut an, um fortzufahren.'
const LAST_FACTOR =
  'Das ist Ihr letzter Faktor: Sie müssen bei der nächsten Anmeldung einen neuen einrichten.'

export const DE_SETTINGS_LABELS = {
  mfaSettings: {
    heading: 'Authentifizierungs-App',
    enrollingHeading: 'Authentifizierungs-App einrichten',
    codesHeading: 'Backup-Codes',
    help: 'Google Authenticator, Authy, 1Password…: Jede TOTP-App funktioniert.',
    codesHelp: 'Bewahren Sie sie auf: Sie werden nur einmal angezeigt.',
    loading: 'Zwei-Faktor-Authentifizierung wird geladen…',
    loadFailed: 'Die Zwei-Faktor-Authentifizierung konnte nicht geladen werden.',
    retry: 'Erneut versuchen',
    enrollLead: 'Bestätigen Sie Ihr Passwort, um die Einrichtung zu beginnen.',
    enrollSubmit: 'Weiter',
    enrollBusy: 'Wird vorbereitet',
    enrollFailed: 'Die Einrichtung konnte nicht gestartet werden, versuchen Sie es erneut.',
    regenerateLead:
      'Bestätigen Sie Ihr Passwort, um 10 neue Codes zu erzeugen. Die bisherigen funktionieren ab sofort nicht mehr.',
    regenerateSubmit: 'Neu erzeugen',
    regenerateBusy: 'Wird neu erzeugt',
    regenerateFailed: 'Die Backup-Codes konnten nicht neu erzeugt werden, versuchen Sie es erneut.',
    disableSubmit: 'Weiter',
    disableCheckBusy: 'Wird geprüft',
    disableFailed:
      'Die Zwei-Faktor-Authentifizierung konnte nicht zurückgesetzt werden, versuchen Sie es erneut.',
    removeTitle: 'Authentifizierungs-App entfernen',
    removeHelp:
      'Entfernt Ihre aktuelle App. Sie werden auf allen Geräten abgemeldet und melden sich danach mit Ihrem Passkey an.',
    removeAction: 'Entfernen',
    removeDialogHeading: 'Authentifizierungs-App entfernen?',
    removeDialogMessage:
      'Sie werden auf allen Geräten abgemeldet. Sie melden sich danach mit Ihrem Passkey an, ohne App.',
    removePromptLead: 'Bestätigen Sie Ihr Passwort, um Ihre Authentifizierungs-App zu entfernen.',
    removeBusy: 'Wird entfernt',
    resetTitle: 'Zwei-Faktor-Authentifizierung zurücksetzen',
    resetHelp:
      'Entfernt Ihre aktuelle App. Sie werden auf allen Geräten abgemeldet und müssen bei der nächsten Anmeldung eine neue einrichten.',
    resetAction: 'Zurücksetzen',
    resetDialogHeading: 'Zwei-Faktor-Authentifizierung zurücksetzen?',
    resetDialogMessage:
      'Sie werden abgemeldet und müssen bei der nächsten Anmeldung eine neue Authentifizierungs-App einrichten.',
    resetPromptLead:
      'Bestätigen Sie Ihr Passwort, um die Zwei-Faktor-Authentifizierung zurückzusetzen.',
    resetBusy: 'Wird zurückgesetzt',
    codesLeft: (count) =>
      count === 0
        ? 'Keine Backup-Codes mehr übrig'
        : count === 1
          ? 'Noch 1 Backup-Code übrig'
          : `Noch ${count} Backup-Codes übrig`,
    lowCodes: 'Erzeugen Sie neue, damit Sie den Zugang zu Ihrem Konto nicht verlieren.',
    scanLead:
      'Scannen Sie diesen QR-Code mit Ihrer Authentifizierungs-App und geben Sie dann den angezeigten 6-stelligen Code ein.',
    codeLabel: '6-stelliger Code',
    activate: 'Aktivieren',
    verifying: 'Wird geprüft',
    cancel: 'Abbrechen',
    close: 'Schließen',
    codesLeadEnrolled:
      'Die Zwei-Faktor-Authentifizierung ist aktiviert. Diese Codes werden nicht erneut angezeigt: Mit ihnen können Sie sich anmelden, wenn Sie keinen Zugriff mehr auf Ihre App haben, und jeder funktioniert nur einmal.',
    codesLeadRegenerated:
      'Hier sind Ihre neuen Backup-Codes. Die bisherigen funktionieren nicht mehr. Diese Codes werden nicht erneut angezeigt, und jeder funktioniert nur einmal.',
    dontLeave: 'Verlassen Sie diese Seite erst, wenn Sie sie gespeichert haben.',
    done: 'Fertig',
    appConfigured: 'App eingerichtet',
    enabled: 'Aktiviert',
    appHelpWithPasskey:
      'Ein 6-stelliger Code aus Ihrer App kann nach Ihrem Passwort Ihren Passkey ersetzen.',
    appHelp: 'Nach Ihrem Passwort wird ein 6-stelliger Code aus Ihrer App abgefragt.',
    noApp: 'Keine App eingerichtet',
    optional: 'Optional',
    noAppHelp:
      'Ihr Passkey genügt für die Anmeldung. Fügen Sie eine App als zweiten Weg hinzu: Sie erzeugt einen 6-stelligen Code.',
    addApp: 'App hinzufügen',
    backupCodesTitle: 'Backup-Codes',
    backupCodesHelp:
      'Jeder Code funktioniert nur einmal. Neue zu erzeugen macht die bisherigen ungültig.',
    regenerateCodes: 'Backup-Codes neu erzeugen',
    noFactorWarning:
      'Die Zwei-Faktor-Authentifizierung ist verpflichtend: Sie müssen sie bei der nächsten Anmeldung einrichten.',
    setUpNow: 'Jetzt einrichten',
    currentPassword: 'Aktuelles Passwort',
    showPassword: 'Passwort anzeigen',
    hidePassword: 'Passwort verbergen',
    enterPassword: 'Geben Sie Ihr Passwort ein',
    wrongPassword: 'Falsches Passwort',
    enterCode: 'Geben Sie den 6-stelligen Code aus Ihrer App ein',
    wrongCode: 'Falscher Code',
    activationFailed: 'Die Aktivierung ist fehlgeschlagen, versuchen Sie es erneut.',
    tooManyAttempts: TOO_MANY_ATTEMPTS,
    signedOut: SIGNED_OUT,
  },
  passkeySettings: {
    heading: 'Passkeys',
    description:
      'Bestätigen Sie Ihre Anmeldung mit Fingerabdruck, Gesicht oder der PIN Ihres Geräts oder mit einem Sicherheitsschlüssel.',
    loading: 'Passkeys werden geladen…',
    loadFailed: 'Die Passkeys konnten nicht geladen werden.',
    retry: 'Erneut versuchen',
    browserBlocked:
      'Dieser Browser unterstützt keine Passkeys: Sie können hier keinen hinzufügen, aber die vorhandenen löschen.',
    serverBlocked: `${PASSKEYS_UNAVAILABLE} Sie können keinen hinzufügen, aber die vorhandenen löschen.`,
    emptyHeading: 'Keine Passkeys',
    emptyMessage:
      'Fügen Sie einen hinzu, um Ihre Anmeldung mit einer Geste zu bestätigen, ohne Code abzutippen.',
    listLabel: 'Registrierte Passkeys',
    added: (absolute) => (absolute ? 'Hinzugefügt am' : 'Hinzugefügt'),
    lastUsed: (absolute) => (absolute ? 'Zuletzt verwendet am' : 'Zuletzt verwendet'),
    neverUsed: 'Nie verwendet',
    delete: 'Löschen',
    deleteKey: (name) => `Schlüssel ${name} löschen`,
    passkeyAdded: (name) => `Passkey „${name}“ hinzugefügt.`,
    passkeyGone: (name) => `Der Passkey „${name}“ existiert nicht mehr.`,
    addPasskey: 'Passkey hinzufügen',
    finishAppFirst: 'Schließen Sie zuerst die Einrichtung der Authentifizierungs-App ab.',
    deleteDialogHeading: 'Diesen Passkey löschen?',
    deleteDialogMessage: (name, lastFactor) =>
      `Mit „${name}“ können Sie sich nicht mehr anmelden. Sie werden auf allen Geräten abgemeldet und müssen sich erneut anmelden.` +
      (lastFactor ? ` ${LAST_FACTOR}` : ''),
    deleteConfirm: 'Löschen',
    cancel: 'Abbrechen',
    close: 'Schließen',
    deleting: 'Wird gelöscht',
    addLead:
      'Benennen Sie diesen Schlüssel und bestätigen Sie Ihr Passwort: Ihr Gerät fragt anschließend nach einer Bestätigung.',
    promptOpen: 'Bestätigen Sie auf Ihrem Gerät, um den Schlüssel zu erstellen.',
    nameLabel: 'Name des Schlüssels (optional)',
    defaultPasskeyName: 'Passkey',
    currentPassword: 'Aktuelles Passwort',
    showPassword: 'Passwort anzeigen',
    hidePassword: 'Passwort verbergen',
    create: 'Passkey erstellen',
    creating: 'Wird erstellt',
    deleteLead: (name) => `Bestätigen Sie Ihr Passwort, um „${name}“ zu löschen.`,
    lastFactorWarning: LAST_FACTOR,
    continue: 'Weiter',
    checking: 'Wird geprüft',
    nameTooLong: (max) => `Der Name darf höchstens ${max} Zeichen lang sein`,
    nameControlCharacters: 'Der Name darf keine Steuerzeichen enthalten',
    enterPassword: 'Geben Sie Ihr Passwort ein',
    wrongPassword: 'Falsches Passwort',
    tooManyPasskeys:
      'Sie haben die Höchstzahl an Passkeys erreicht: Löschen Sie einen, bevor Sie einen weiteren hinzufügen.',
    alreadyRegistered: 'Dieser Schlüssel ist bereits registriert',
    passkeysUnavailable: PASSKEYS_UNAVAILABLE,
    addFailed: 'Der Passkey konnte nicht erstellt werden, versuchen Sie es erneut.',
    cancelled: 'Vorgang abgebrochen',
    browserUnsupported: 'Dieser Browser unterstützt keine Passkeys.',
    deleteFailed: 'Der Passkey konnte nicht gelöscht werden, versuchen Sie es erneut.',
    tooManyAttempts: TOO_MANY_ATTEMPTS,
    signedOut: SIGNED_OUT,
  },
} satisfies SettingsLabels
