import type { StorybookConfig } from '@storybook/angular-vite'
import path from 'node:path'
import fs from 'node:fs'
import { fileURLToPath } from 'node:url'
import type { Plugin } from 'vite'

const dirname =
  typeof __dirname !== 'undefined' ? __dirname : path.dirname(fileURLToPath(import.meta.url))

/**
 * The logo and favicon come from a backend route Storybook does not have: serve the default files
 * instead.
 */
function mockBrandingApi(): Plugin {
  const assetsDir = path.resolve(dirname, '../../crates/artiferris-infrastructure/assets')
  const routes: Record<string, { file: string; contentType: string }> = {
    '/api/branding/logo': { file: 'artiferris-logo.png', contentType: 'image/png' },
    '/api/branding/favicon': { file: 'artiferris-favicon.ico', contentType: 'image/x-icon' },
  }
  return {
    name: 'mock-branding-api',
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const route = req.url && routes[req.url]
        if (!route) {
          next()
          return
        }
        res.setHeader('Content-Type', route.contentType)
        fs.createReadStream(path.join(assetsDir, route.file)).pipe(res)
      })
    },
  }
}

const config: StorybookConfig = {
  stories: ['../src/**/*.mdx', '../src/**/*.stories.@(js|jsx|mjs|ts|tsx)'],
  addons: [
    '@chromatic-com/storybook',
    '@storybook/addon-vitest',
    '@storybook/addon-a11y',
    '@storybook/addon-docs',
  ],
  framework: '@storybook/angular-vite',
  // The app's public/ folder, at the paths the app uses.
  staticDirs: ['../public'],
  async viteFinal(viteConfig) {
    viteConfig.plugins ??= []
    viteConfig.plugins.push(mockBrandingApi())
    return viteConfig
  },
}
export default config
