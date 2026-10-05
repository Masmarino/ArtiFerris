import type { AuthLabels } from '@masmarino/gabarit/auth'

/** Over the kit's English defaults: what names ArtiFerris and its files. */
export const EN_AUTH_LABELS = {
  backupCodes: {
    fileName: 'artiferris-backup-codes.txt',
    fileTitle: 'ArtiFerris - backup codes',
  },
  login: {
    intro: 'Sign in to find your repositories and packages.',
  },
  register: {
    intro: 'Join ArtiFerris to publish and share your packages.',
    closedMessage:
      'This organization does not accept open registration. Ask an administrator for an invitation: you will receive a link by email to activate your account.',
  },
  activate: {
    invalidMessage:
      'This activation link is invalid or has expired. Ask an administrator to send you a new invitation.',
  },
  resetPassword: {
    intro:
      'An administrator reset the password of your account: your old password no longer works. Choose a new one to sign in again.',
    invalidMessage:
      'This reset link is invalid, was already used or has expired. Ask an administrator to start again.',
  },
} satisfies AuthLabels
