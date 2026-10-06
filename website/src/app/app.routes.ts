import { Routes } from '@angular/router'
import { defaultLangResolver, langMatch, langResolver } from './i18n/lang-routing'

export const routes: Routes = [
  {
    path: ':lang',
    canMatch: [langMatch],
    resolve: { lang: langResolver },
    loadComponent: () => import('./layout/lang-shell/lang-shell').then((m) => m.LangShell),
    children: [
      {
        path: '',
        loadComponent: () => import('./pages/home/home').then((m) => m.Home),
      },
      {
        path: 'features',
        loadComponent: () => import('./pages/features/features').then((m) => m.Features),
      },
      {
        path: 'registries',
        loadComponent: () => import('./pages/registries/registries').then((m) => m.Registries),
      },
      {
        path: 'install',
        loadComponent: () => import('./pages/install/install').then((m) => m.InstallPage),
      },
      {
        path: 'security',
        loadComponent: () => import('./pages/security/security').then((m) => m.Security),
      },
      {
        path: 'roadmap',
        loadComponent: () => import('./pages/roadmap/roadmap').then((m) => m.Roadmap),
      },
    ],
  },
  {
    // Only here to be prerendered: finalize-dist.mjs turns the output into 404.html.
    path: '404',
    resolve: { lang: defaultLangResolver },
    loadComponent: () => import('./pages/not-found/not-found').then((m) => m.NotFound),
  },
  {
    // Unknown languages and pages end here. nginx redirects `/` before it gets this far.
    path: '**',
    resolve: { lang: defaultLangResolver },
    loadComponent: () => import('./pages/not-found/not-found').then((m) => m.NotFound),
  },
]
