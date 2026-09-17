import { applicationConfig, type Preview } from '@storybook/angular-vite'
// The app's own global stylesheet — fonts, CSS custom properties (--primary, --bg-principal, …)
// and resets. Without it, every component renders with browser-default fonts/colors, since
// Storybook's preview iframe never loads the real app's index.html/styles.scss on its own.
import '../src/styles.scss'
import { provideArtiferrisIcons } from '../src/app/shared/register-icons'

const preview: Preview = {
  // Registers ArtiFerris's own icon set (package, users, server, …) — gbt-icon only ships a
  // handful of built-in names itself, same as the real app.config.ts, or every gbt-icon using
  // one of ours silently renders nothing (aria-hidden, so no story assertion catches it).
  decorators: [applicationConfig({ providers: [provideArtiferrisIcons()] })],
  parameters: {
    controls: {
      matchers: {
        color: /(background|color)$/i,
        date: /Date$/i,
      },
    },

    a11y: {
      // 'todo' - show a11y violations in the test UI only
      // 'error' - fail CI on a11y violations
      // 'off' - skip a11y checks entirely
      test: 'todo',
    },
  },
}

export default preview
