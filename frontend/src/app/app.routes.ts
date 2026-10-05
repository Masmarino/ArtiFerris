import { inject } from '@angular/core'
import { Routes } from '@angular/router'
import { AuthService } from './auth/application/auth.service'
import { authGuard } from './auth/auth.guard'
import { adminGuard } from './auth/admin.guard'
import { usersGuard } from './auth/users.guard'
import { organizationAdminGuard } from './auth/organization-admin.guard'
import { CATALOGS } from './public/catalog/domain/catalog.registry'
import { personalOwnerMatcher } from './public/catalog/owner-url-matcher'
import { ADMIN_TRAIL, ADMINISTRATION_TRAIL } from './shell/page-trail'
import { knownFormatAt } from './public/known-format'

const catalogRoutes: Routes = CATALOGS.map((catalog) => ({
  path: catalog.name,
  loadComponent: () =>
    import('./public/catalog/catalog-page/catalog-page').then((m) => m.CatalogPage),
  data: { catalogName: catalog.name, format: catalog.format },
}))

export const routes: Routes = [
  {
    path: 'login',
    loadComponent: () => import('./auth/login-page/login-page').then((m) => m.LoginPage),
  },
  {
    path: 'activate',
    loadComponent: () => import('./auth/activate-page/activate-page').then((m) => m.ActivatePage),
  },
  {
    path: 'reset-password',
    loadComponent: () =>
      import('./auth/reset-password-page/reset-password-page').then((m) => m.ResetPasswordPage),
  },
  {
    path: 'register',
    loadComponent: () => import('./auth/register-page/register-page').then((m) => m.RegisterPage),
  },
  {
    path: 'explorer',
    loadComponent: () =>
      import('./public/catalog/explorer-page/explorer-page').then((m) => m.ExplorerPage),
  },
  // The home page. It must stay ahead of AppShell, whose own '' would claim it.
  {
    path: '',
    pathMatch: 'full',
    loadComponent: () =>
      import('./public/catalog/explorer-page/explorer-page').then((m) => m.ExplorerPage),
  },
  ...catalogRoutes,
  // The documentation without a session, under the public pages' bar; with one, it is in the shell below.
  {
    path: 'docs',
    canMatch: [() => !inject(AuthService).isAuthenticated()],
    loadChildren: () => import('./docs/docs.routes').then((m) => m.PUBLIC_DOCS_ROUTES),
  },
  // Authenticated routes never start with '@'.
  {
    matcher: personalOwnerMatcher,
    loadComponent: () => import('./public/catalog/owner-page/owner-page').then((m) => m.OwnerPage),
  },
  {
    path: '',
    loadComponent: () => import('./shell/app-shell').then((m) => m.AppShell),
    data: { recreatesViewsOnLanguageChange: true },
    canActivate: [authGuard],
    children: [
      {
        path: 'users',
        loadComponent: () => import('./users/users-list/users-list').then((m) => m.UsersList),
        canActivate: [usersGuard],
        data: { trail: ADMINISTRATION_TRAIL, titleKey: 'nav.users' },
      },
      {
        path: 'users/:id',
        loadComponent: () => import('./users/user-detail/user-detail').then((m) => m.UserDetail),
        canActivate: [usersGuard],
        data: { trail: [...ADMINISTRATION_TRAIL, { labelKey: 'nav.users', link: '/users' }] },
      },
      {
        path: 'docs',
        loadChildren: () => import('./docs/docs.routes').then((m) => m.SHELL_DOCS_ROUTES),
      },
      {
        path: 'account',
        loadComponent: () =>
          import('./account/account-page/account-page').then((m) => m.AccountPage),
        data: { titleKey: 'nav.account' },
      },
      {
        path: 'repositories',
        loadComponent: () =>
          import('./repositories/repositories-list/repositories-list').then(
            (m) => m.RepositoriesList,
          ),
        data: { titleKey: 'nav.repositories' },
      },
      {
        path: 'my-repository',
        loadComponent: () =>
          import('./repositories/my-repository-page/my-repository-page').then(
            (m) => m.MyRepositoryPage,
          ),
        data: { titleKey: 'nav.myRepository' },
      },
      {
        path: 'repositories/:id',
        loadComponent: () =>
          import('./repositories/repository-detail/repository-detail').then(
            (m) => m.RepositoryDetail,
          ),
        data: { trail: [{ labelKey: 'nav.repositories', link: '/repositories' }] },
      },
      {
        path: 'repositories/:id/packages/:format/:name',
        canMatch: [knownFormatAt(3)],
        loadComponent: () =>
          import('./repositories/package-detail-page/package-detail-page').then(
            (m) => m.PackageDetailPage,
          ),
        data: { trail: [{ labelKey: 'nav.repositories', link: '/repositories' }] },
      },
      {
        path: 'admin',
        loadComponent: () =>
          import('./admin/admin-dashboard/admin-dashboard').then((m) => m.AdminDashboard),
        canActivate: [adminGuard],
        data: { trail: ADMINISTRATION_TRAIL, titleKey: 'nav.dashboard' },
      },
      {
        path: 'admin/export',
        loadComponent: () => import('./admin/export/export').then((m) => m.ExportAdmin),
        canActivate: [adminGuard],
        data: { trail: ADMIN_TRAIL, titleKey: 'nav.export' },
      },
      {
        path: 'admin/health',
        loadComponent: () =>
          import('./admin/health-status/health-status').then((m) => m.HealthStatusPage),
        canActivate: [adminGuard],
        data: { trail: ADMIN_TRAIL, titleKey: 'nav.health' },
      },
      {
        path: 'admin/organizations',
        loadComponent: () =>
          import('./admin/organizations-page/organizations-page').then((m) => m.OrganizationsPage),
        canActivate: [adminGuard],
        data: { trail: ADMIN_TRAIL, titleKey: 'nav.organizations' },
      },
      {
        path: 'admin/organizations/:id',
        loadComponent: () =>
          import('./admin/organizations-page/organizations-page').then((m) => m.OrganizationsPage),
        canActivate: [organizationAdminGuard],
        data: {
          trail: [...ADMIN_TRAIL, { labelKey: 'nav.organizations', link: '/admin/organizations' }],
        },
      },
    ],
  },
  // After AppShell: as leaf routes with wildcard params they would match any 2- or 5-segment URL
  // first.
  // 'o/:slug' stays ahead of ':username/:repoName', which would swallow it.
  {
    path: 'o/:slug',
    loadComponent: () => import('./public/catalog/owner-page/owner-page').then((m) => m.OwnerPage),
  },
  {
    path: ':username/:repoName',
    loadComponent: () =>
      import('./public/public-repository-page/public-repository-page').then(
        (m) => m.PublicRepositoryPage,
      ),
  },
  {
    path: ':username/:repoName/packages/:format/:name',
    canMatch: [knownFormatAt(3)],
    loadComponent: () =>
      import('./public/public-package-page/public-package-page').then((m) => m.PublicPackagePage),
  },
  {
    path: 'o/:slug/:repoName',
    loadComponent: () =>
      import('./public/public-repository-page/public-repository-page').then(
        (m) => m.PublicRepositoryPage,
      ),
  },
  {
    path: 'o/:slug/:repoName/packages/:format/:name',
    canMatch: [knownFormatAt(4)],
    loadComponent: () =>
      import('./public/public-package-page/public-package-page').then((m) => m.PublicPackagePage),
  },
  {
    path: '**',
    loadComponent: () => import('./not-found/not-found-page').then((m) => m.NotFoundPage),
  },
]
