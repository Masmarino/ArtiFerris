import type { SettingsLabels } from './kit-labels'

const TOO_MANY_ATTEMPTS = 'Troppi tentativi, riprova tra qualche minuto'
const PASSKEYS_UNAVAILABLE = 'Le passkey non sono disponibili su questo server.'
const SIGNED_OUT = 'Sei stato disconnesso. Accedi di nuovo per continuare.'
const LAST_FACTOR = 'È il tuo ultimo fattore: dovrai configurarne uno nuovo al prossimo accesso.'

export const IT_SETTINGS_LABELS = {
  mfaSettings: {
    heading: 'App di autenticazione',
    enrollingHeading: "Configura l'app di autenticazione",
    codesHeading: 'Codici di backup',
    help: 'Google Authenticator, Authy, 1Password…: va bene qualsiasi app TOTP.',
    codesHelp: 'Conservali: vengono mostrati una sola volta.',
    loading: "Caricamento dell'autenticazione a due fattori…",
    loadFailed: "Impossibile caricare l'autenticazione a due fattori.",
    retry: 'Riprova',
    enrollLead: 'Conferma la tua password per iniziare la configurazione.',
    enrollSubmit: 'Continua',
    enrollBusy: 'Preparazione in corso',
    enrollFailed: 'Impossibile avviare la configurazione, riprova.',
    regenerateLead:
      'Conferma la tua password per generare 10 nuovi codici. I precedenti smetteranno subito di funzionare.',
    regenerateSubmit: 'Rigenera',
    regenerateBusy: 'Rigenerazione in corso',
    regenerateFailed: 'Impossibile rigenerare i codici di backup, riprova.',
    disableSubmit: 'Continua',
    disableCheckBusy: 'Verifica in corso',
    disableFailed: "Impossibile reimpostare l'autenticazione a due fattori, riprova.",
    removeTitle: "Rimuovi l'app di autenticazione",
    removeHelp:
      'Rimuove la tua app attuale. Verrai disconnesso da tutti i tuoi dispositivi e accederai di nuovo con la tua passkey.',
    removeAction: 'Rimuovi',
    removeDialogHeading: "Rimuovere l'app di autenticazione?",
    removeDialogMessage:
      'Verrai disconnesso da tutti i tuoi dispositivi. Accederai di nuovo con la tua passkey, senza app.',
    removePromptLead: 'Conferma la tua password per rimuovere la tua app di autenticazione.',
    removeBusy: 'Rimozione in corso',
    resetTitle: "Reimposta l'autenticazione a due fattori",
    resetHelp:
      'Rimuove la tua app attuale. Verrai disconnesso da tutti i tuoi dispositivi e dovrai configurarne una nuova al prossimo accesso.',
    resetAction: 'Reimposta',
    resetDialogHeading: "Reimpostare l'autenticazione a due fattori?",
    resetDialogMessage:
      'Verrai disconnesso e dovrai configurare una nuova app di autenticazione al prossimo accesso.',
    resetPromptLead: "Conferma la tua password per reimpostare l'autenticazione a due fattori.",
    resetBusy: 'Reimpostazione in corso',
    codesLeft: (count) =>
      count === 0
        ? 'Nessun codice di backup rimasto'
        : count === 1
          ? '1 codice di backup rimasto'
          : `${count} codici di backup rimasti`,
    lowCodes: "Generane di nuovi per non perdere l'accesso al tuo account.",
    scanLead:
      'Scansiona questo codice QR con la tua app di autenticazione, poi inserisci il codice di 6 cifre che mostra.',
    codeLabel: 'Codice di 6 cifre',
    activate: 'Attiva',
    verifying: 'Verifica in corso',
    cancel: 'Annulla',
    close: 'Chiudi',
    codesLeadEnrolled:
      "L'autenticazione a due fattori è attiva. Questi codici non verranno più mostrati: ti permettono di accedere se perdi l'accesso alla tua app, e ciascuno funziona una sola volta.",
    codesLeadRegenerated:
      'Ecco i tuoi nuovi codici di backup. I precedenti non funzionano più. Questi codici non verranno più mostrati, e ciascuno funziona una sola volta.',
    dontLeave: 'Non lasciare questa pagina prima di averli salvati.',
    done: 'Fatto',
    appConfigured: 'App configurata',
    enabled: 'Attiva',
    appHelpWithPasskey:
      'Un codice di 6 cifre, generato dalla tua app, può sostituire la tua passkey dopo la password.',
    appHelp: 'Dopo la password viene chiesto un codice di 6 cifre, generato dalla tua app.',
    noApp: 'Nessuna app configurata',
    optional: 'Facoltativa',
    noAppHelp:
      "La tua passkey basta per accedere. Aggiungi un'app per avere una seconda via: genera un codice di 6 cifre.",
    addApp: "Aggiungi un'app",
    backupCodesTitle: 'Codici di backup',
    backupCodesHelp:
      'Ogni codice funziona una sola volta. Generarne di nuovi invalida i precedenti.',
    regenerateCodes: 'Rigenera i codici di backup',
    noFactorWarning:
      "L'autenticazione a due fattori è obbligatoria: dovrai configurarla al prossimo accesso.",
    setUpNow: 'Configura ora',
    currentPassword: 'Password attuale',
    showPassword: 'Mostra la password',
    hidePassword: 'Nascondi la password',
    enterPassword: 'Inserisci la tua password',
    wrongPassword: 'Password errata',
    enterCode: 'Inserisci il codice di 6 cifre della tua app',
    wrongCode: 'Codice errato',
    activationFailed: 'Attivazione non riuscita, riprova.',
    tooManyAttempts: TOO_MANY_ATTEMPTS,
    signedOut: SIGNED_OUT,
  },
  passkeySettings: {
    heading: 'Passkey',
    description:
      "Conferma l'accesso con l'impronta, il volto o il PIN del tuo dispositivo, oppure con una chiave di sicurezza.",
    loading: 'Caricamento delle passkey…',
    loadFailed: 'Impossibile caricare le passkey.',
    retry: 'Riprova',
    browserBlocked:
      'Questo browser non supporta le passkey: non puoi aggiungerne qui, ma puoi eliminare quelle che hai già.',
    serverBlocked: `${PASSKEYS_UNAVAILABLE} Non puoi aggiungerne, ma puoi eliminare quelle che hai già.`,
    emptyHeading: 'Nessuna passkey',
    emptyMessage: "Aggiungine una per confermare l'accesso con un gesto, senza codici da copiare.",
    listLabel: 'Passkey registrate',
    added: (absolute) => (absolute ? 'Aggiunta il' : 'Aggiunta'),
    lastUsed: (absolute) => (absolute ? 'Ultimo utilizzo il' : 'Ultimo utilizzo'),
    neverUsed: 'Mai usata',
    delete: 'Elimina',
    deleteKey: (name) => `Elimina la chiave ${name}`,
    passkeyAdded: (name) => `Passkey «${name}» aggiunta.`,
    passkeyGone: (name) => `La passkey «${name}» non esiste più.`,
    addPasskey: 'Aggiungi una passkey',
    finishAppFirst: "Completa prima la configurazione dell'app di autenticazione.",
    deleteDialogHeading: 'Eliminare questa passkey?',
    deleteDialogMessage: (name, lastFactor) =>
      `«${name}» non ti permetterà più di accedere. Verrai disconnesso da tutti i tuoi dispositivi e dovrai accedere di nuovo.` +
      (lastFactor ? ` ${LAST_FACTOR}` : ''),
    deleteConfirm: 'Elimina',
    cancel: 'Annulla',
    close: 'Chiudi',
    deleting: 'Eliminazione in corso',
    addLead:
      'Dai un nome a questa chiave e conferma la tua password: il tuo dispositivo ti chiederà poi di confermare.',
    promptOpen: 'Conferma sul tuo dispositivo per creare la chiave.',
    nameLabel: 'Nome della chiave (facoltativo)',
    defaultPasskeyName: 'Passkey',
    currentPassword: 'Password attuale',
    showPassword: 'Mostra la password',
    hidePassword: 'Nascondi la password',
    create: 'Crea la passkey',
    creating: 'Creazione in corso',
    deleteLead: (name) => `Conferma la tua password per eliminare «${name}».`,
    lastFactorWarning: LAST_FACTOR,
    continue: 'Continua',
    checking: 'Verifica in corso',
    nameTooLong: (max) => `Il nome non deve superare i ${max} caratteri`,
    nameControlCharacters: 'Il nome non può contenere caratteri di controllo',
    enterPassword: 'Inserisci la tua password',
    wrongPassword: 'Password errata',
    tooManyPasskeys:
      "Hai raggiunto il numero massimo di passkey: eliminane una prima di aggiungerne un'altra.",
    alreadyRegistered: 'Questa chiave è già registrata',
    passkeysUnavailable: PASSKEYS_UNAVAILABLE,
    addFailed: 'Impossibile creare la passkey, riprova.',
    cancelled: 'Operazione annullata',
    browserUnsupported: 'Questo browser non supporta le passkey.',
    deleteFailed: 'Impossibile eliminare la passkey, riprova.',
    tooManyAttempts: TOO_MANY_ATTEMPTS,
    signedOut: SIGNED_OUT,
  },
} satisfies SettingsLabels
