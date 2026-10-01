import { applicationConfig, type Preview } from '@storybook/angular-vite'
// The app's global stylesheet: fonts, colors and resets.
import '../src/styles.scss'
import { provideArtiferrisIcons } from '../src/app/shared/register-icons'
import { provideStorybookTransloco } from './transloco'

const preview: Preview = {
  // Register our icons as app.config.ts does, or they render empty.
  decorators: [
    applicationConfig({ providers: [provideArtiferrisIcons(), ...provideStorybookTransloco()] }),
  ],
  parameters: {
    controls: {
      matchers: {
        color: /(background|color)$/i,
        date: /Date$/i,
      },
    },

    a11y: {
      // a11y violations: 'todo' shows them in the test UI, 'error' fails CI, 'off' skips the
      // checks.
      test: 'todo',
    },
  },
}

export default preview
