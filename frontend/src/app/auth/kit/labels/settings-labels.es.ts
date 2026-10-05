import type { SettingsLabels } from './kit-labels'

const TOO_MANY_ATTEMPTS = 'Demasiados intentos, inténtelo de nuevo dentro de unos minutos'
const PASSKEYS_UNAVAILABLE = 'Las claves de acceso no están disponibles en este servidor.'
const SIGNED_OUT = 'Se ha cerrado su sesión. Vuelva a iniciar sesión para continuar.'
const LAST_FACTOR =
  'Es su último factor: tendrá que configurar uno nuevo en su próximo inicio de sesión.'

export const ES_SETTINGS_LABELS = {
  mfaSettings: {
    heading: 'Aplicación de autenticación',
    enrollingHeading: 'Configurar la aplicación de autenticación',
    codesHeading: 'Códigos de respaldo',
    help: 'Google Authenticator, Authy, 1Password…: cualquier aplicación TOTP sirve.',
    codesHelp: 'Guárdelos: solo se muestran una vez.',
    loading: 'Cargando la autenticación de dos factores…',
    loadFailed: 'No se ha podido cargar la autenticación de dos factores.',
    retry: 'Reintentar',
    enrollLead: 'Confirme su contraseña para empezar la configuración.',
    enrollSubmit: 'Continuar',
    enrollBusy: 'Preparando',
    enrollFailed: 'No se ha podido iniciar la configuración, inténtelo de nuevo.',
    regenerateLead:
      'Confirme su contraseña para generar 10 códigos nuevos. Los anteriores dejarán de funcionar de inmediato.',
    regenerateSubmit: 'Regenerar',
    regenerateBusy: 'Regenerando',
    regenerateFailed: 'No se han podido regenerar los códigos de respaldo, inténtelo de nuevo.',
    disableSubmit: 'Continuar',
    disableCheckBusy: 'Comprobando',
    disableFailed:
      'No se ha podido restablecer la autenticación de dos factores, inténtelo de nuevo.',
    removeTitle: 'Eliminar la aplicación de autenticación',
    removeHelp:
      'Elimina su aplicación actual. Se cerrará su sesión en todos sus dispositivos y volverá a iniciarla con su clave de acceso.',
    removeAction: 'Eliminar',
    removeDialogHeading: '¿Eliminar la aplicación de autenticación?',
    removeDialogMessage:
      'Se cerrará su sesión en todos sus dispositivos. Volverá a iniciarla con su clave de acceso, sin aplicación.',
    removePromptLead: 'Confirme su contraseña para eliminar su aplicación de autenticación.',
    removeBusy: 'Eliminando',
    resetTitle: 'Restablecer la autenticación de dos factores',
    resetHelp:
      'Elimina su aplicación actual. Se cerrará su sesión en todos sus dispositivos y tendrá que configurar una nueva en su próximo inicio de sesión.',
    resetAction: 'Restablecer',
    resetDialogHeading: '¿Restablecer la autenticación de dos factores?',
    resetDialogMessage:
      'Se cerrará su sesión y tendrá que configurar una nueva aplicación de autenticación en su próximo inicio de sesión.',
    resetPromptLead: 'Confirme su contraseña para restablecer la autenticación de dos factores.',
    resetBusy: 'Restableciendo',
    codesLeft: (count) =>
      count === 0
        ? 'No queda ningún código de respaldo'
        : count === 1
          ? 'Queda 1 código de respaldo'
          : `Quedan ${count} códigos de respaldo`,
    lowCodes: 'Genere otros nuevos para no perder el acceso a su cuenta.',
    scanLead:
      'Escanee este código QR con su aplicación de autenticación y luego introduzca el código de 6 cifras que muestra.',
    codeLabel: 'Código de 6 cifras',
    activate: 'Activar',
    verifying: 'Verificando',
    cancel: 'Cancelar',
    close: 'Cerrar',
    codesLeadEnrolled:
      'La autenticación de dos factores está activada. Estos códigos no se volverán a mostrar: le permiten iniciar sesión si pierde el acceso a su aplicación, y cada uno solo funciona una vez.',
    codesLeadRegenerated:
      'Estos son sus nuevos códigos de respaldo. Los anteriores ya no funcionan. Estos códigos no se volverán a mostrar, y cada uno solo funciona una vez.',
    dontLeave: 'No salga de esta página antes de haberlos guardado.',
    done: 'Hecho',
    appConfigured: 'Aplicación configurada',
    enabled: 'Activada',
    appHelpWithPasskey:
      'Un código de 6 cifras, generado por su aplicación, puede sustituir a su clave de acceso después de su contraseña.',
    appHelp: 'Se pide un código de 6 cifras, generado por su aplicación, después de su contraseña.',
    noApp: 'Ninguna aplicación configurada',
    optional: 'Opcional',
    noAppHelp:
      'Su clave de acceso basta para iniciar sesión. Añada una aplicación para disponer de una segunda vía: genera un código de 6 cifras.',
    addApp: 'Añadir una aplicación',
    backupCodesTitle: 'Códigos de respaldo',
    backupCodesHelp:
      'Cada código solo funciona una vez. Generar otros nuevos invalida los anteriores.',
    regenerateCodes: 'Regenerar los códigos de respaldo',
    noFactorWarning:
      'La autenticación de dos factores es obligatoria: tendrá que configurarla en su próximo inicio de sesión.',
    setUpNow: 'Configurar ahora',
    currentPassword: 'Contraseña actual',
    showPassword: 'Mostrar la contraseña',
    hidePassword: 'Ocultar la contraseña',
    enterPassword: 'Introduzca su contraseña',
    wrongPassword: 'Contraseña incorrecta',
    enterCode: 'Introduzca el código de 6 cifras de su aplicación',
    wrongCode: 'Código incorrecto',
    activationFailed: 'La activación ha fallado, inténtelo de nuevo.',
    tooManyAttempts: TOO_MANY_ATTEMPTS,
    signedOut: SIGNED_OUT,
  },
  passkeySettings: {
    heading: 'Claves de acceso',
    description:
      'Confirme su inicio de sesión con la huella, el rostro o el PIN de su dispositivo, o con una llave de seguridad.',
    loading: 'Cargando las claves de acceso…',
    loadFailed: 'No se han podido cargar las claves de acceso.',
    retry: 'Reintentar',
    browserBlocked:
      'Este navegador no admite claves de acceso: no puede añadir ninguna aquí, pero puede eliminar las que ya tiene.',
    serverBlocked: `${PASSKEYS_UNAVAILABLE} No puede añadir ninguna, pero puede eliminar las que ya tiene.`,
    emptyHeading: 'Ninguna clave de acceso',
    emptyMessage:
      'Añada una para confirmar su inicio de sesión con un gesto, sin código que copiar.',
    listLabel: 'Claves de acceso registradas',
    added: (absolute) => (absolute ? 'Añadida el' : 'Añadida'),
    lastUsed: (absolute) => (absolute ? 'Último uso el' : 'Último uso'),
    neverUsed: 'Nunca usada',
    delete: 'Eliminar',
    deleteKey: (name) => `Eliminar la clave ${name}`,
    passkeyAdded: (name) => `Clave de acceso «${name}» añadida.`,
    passkeyGone: (name) => `La clave de acceso «${name}» ya no existe.`,
    addPasskey: 'Añadir una clave de acceso',
    finishAppFirst: 'Termine primero la configuración de la aplicación de autenticación.',
    deleteDialogHeading: '¿Eliminar esta clave de acceso?',
    deleteDialogMessage: (name, lastFactor) =>
      `«${name}» ya no le permitirá iniciar sesión. Se cerrará su sesión en todos sus dispositivos y tendrá que volver a iniciarla.` +
      (lastFactor ? ` ${LAST_FACTOR}` : ''),
    deleteConfirm: 'Eliminar',
    cancel: 'Cancelar',
    close: 'Cerrar',
    deleting: 'Eliminando',
    addLead:
      'Ponga un nombre a esta clave y confirme su contraseña: su dispositivo le pedirá después que confirme.',
    promptOpen: 'Confirme en su dispositivo para crear la clave.',
    nameLabel: 'Nombre de la clave (opcional)',
    defaultPasskeyName: 'Clave de acceso',
    currentPassword: 'Contraseña actual',
    showPassword: 'Mostrar la contraseña',
    hidePassword: 'Ocultar la contraseña',
    create: 'Crear la clave de acceso',
    creating: 'Creando',
    deleteLead: (name) => `Confirme su contraseña para eliminar «${name}».`,
    lastFactorWarning: LAST_FACTOR,
    continue: 'Continuar',
    checking: 'Comprobando',
    nameTooLong: (max) => `El nombre no debe superar los ${max} caracteres`,
    nameControlCharacters: 'El nombre no puede contener caracteres de control',
    enterPassword: 'Introduzca su contraseña',
    wrongPassword: 'Contraseña incorrecta',
    tooManyPasskeys:
      'Ha alcanzado el número máximo de claves de acceso: elimine una antes de añadir otra.',
    alreadyRegistered: 'Esta clave ya está registrada',
    passkeysUnavailable: PASSKEYS_UNAVAILABLE,
    addFailed: 'No se ha podido crear la clave de acceso, inténtelo de nuevo.',
    cancelled: 'Operación cancelada',
    browserUnsupported: 'Este navegador no admite claves de acceso.',
    deleteFailed: 'No se ha podido eliminar la clave de acceso, inténtelo de nuevo.',
    tooManyAttempts: TOO_MANY_ATTEMPTS,
    signedOut: SIGNED_OUT,
  },
} satisfies SettingsLabels
