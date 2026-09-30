import { Routes } from '@angular/router'
import { authGuard } from './auth/auth.guard'
import { adminGuard } from './auth/admin.guard'
import { usersGuard } from './auth/users.guard'
import { organizationAdminGuard } from './auth/organization-admin.guard'
import { CATALOGS } from './public/catalog/domain/catalog.registry'
import { personalOwnerMatcher } from './public/catalog/owner-url-matcher'

// One route per catalog: single-segment literals, so they're safe ahead of AppShell.
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
    path: 'register',
    loadComponent: () => import('./auth/register-page/register-page').then((m) => m.RegisterPage),
  },
  {
    path: 'explorer',
    loadComponent: () =>
      import('./public/catalog/explorer-page/explorer-page').then((m) => m.ExplorerPage),
  },
  // The site's home page, reachable to anyone signed in or not (see 'explorer' above, same
  // component): must stay ahead of AppShell below, whose own '' would otherwise claim it first.
  {
    path: '',
    pathMatch: 'full',
    loadComponent: () =>
      import('./public/catalog/explorer-page/explorer-page').then((m) => m.ExplorerPage),
  },
  ...catalogRoutes,
  // Authenticated single-segment routes never start with '@', so this can't shadow any.
  {
    matcher: personalOwnerMatcher,
    loadComponent: () => import('./public/catalog/owner-page/owner-page').then((m) => m.OwnerPage),
  },
  {
    path: '',
    loadComponent: () => import('./shell/app-shell').then((m) => m.AppShell),
    canActivate: [authGuard],
    children: [
      {
        path: 'users',
        loadComponent: () => import('./users/users-list/users-list').then((m) => m.UsersList),
        canActivate: [usersGuard],
        data: { titleKey: 'nav.users' },
      },
      {
        path: 'users/:id',
        loadComponent: () => import('./users/user-detail/user-detail').then((m) => m.UserDetail),
        canActivate: [usersGuard],
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
      },
      {
        path: 'repositories/:id/packages/:format/:name',
        loadComponent: () =>
          import('./repositories/package-detail-page/package-detail-page').then(
            (m) => m.PackageDetailPage,
          ),
      },
      {
        path: 'admin',
        loadComponent: () =>
          import('./admin/admin-dashboard/admin-dashboard').then((m) => m.AdminDashboard),
        canActivate: [adminGuard],
        data: { titleKey: 'nav.administration' },
      },
      {
        path: 'admin/export',
        loadComponent: () => import('./admin/export/export').then((m) => m.ExportAdmin),
        canActivate: [adminGuard],
        data: { titleKey: 'nav.export' },
      },
      {
        path: 'admin/health',
        loadComponent: () =>
          import('./admin/health-status/health-status').then((m) => m.HealthStatusPage),
        canActivate: [adminGuard],
        data: { titleKey: 'nav.systemHealth' },
      },
      {
        path: 'admin/organizations',
        loadComponent: () =>
          import('./admin/organizations-page/organizations-page').then((m) => m.OrganizationsPage),
        canActivate: [adminGuard],
        data: { titleKey: 'nav.organizations' },
      },
      {
        path: 'admin/organizations/:id',
        loadComponent: () =>
          import('./admin/organizations-page/organizations-page').then((m) => m.OrganizationsPage),
        canActivate: [organizationAdminGuard],
      },
    ],
  },
  // These must come AFTER the AppShell entry above: as leaf routes with wildcard params,
  // they'd otherwise match any 2- or 5-segment URL outright (e.g. /repositories/:id,
  // /users/:id, /admin/export) before the router ever gets to try AppShell's own children.
  // 'o/:slug' must also stay ahead of ':username/:repoName', which would swallow it.
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
    loadComponent: () =>
      import('./public/public-package-page/public-package-page').then((m) => m.PublicPackagePage),
  },
  // Keep last.
  {
    path: '**',
    loadComponent: () => import('./not-found/not-found-page').then((m) => m.NotFoundPage),
  },
]
