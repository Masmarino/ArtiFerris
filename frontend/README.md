# ArtiFerrisWeb

This project was generated using [Angular CLI](https://github.com/angular/angular-cli) version 22.1.4.

## Development server

To start a local development server, run:

```bash
ng serve
```

Once the server is running, open your browser and navigate to `http://localhost:4200/`. The application will automatically reload whenever you modify any of the source files.

## Code scaffolding

Angular CLI includes powerful code scaffolding tools. To generate a new component, run:

```bash
ng generate component component-name
```

For a complete list of available schematics (such as `components`, `directives`, or `pipes`), run:

```bash
ng generate --help
```

## Building

To build the project run:

```bash
ng build
```

This will compile your project and store the build artifacts in the `dist/` directory. By default, the production build optimizes your application for performance and speed.

## Running unit tests

To execute unit tests with the [Vitest](https://vitest.dev/) test runner, use the following command:

```bash
ng test
```

## Running end-to-end tests

For end-to-end (e2e) testing, run:

```bash
ng e2e
```

Angular CLI does not come with an end-to-end testing framework by default. You can choose one that suits your needs.

## Known issues

### npm audit — devDependency-only UUID vulnerability (B-44)

`npm audit` reports 5 moderate findings in a devDependency chain (`uuid <11.1.1` via `sockjs` → `webpack-dev-server` → `@angular-devkit/build-angular`). These never reach production (`npm audit --omit=dev` reports 0 findings). No fix is currently available upstream: `sockjs` has not released a version compatible with `uuid ≥ 11.1.1`, and `@angular-devkit/build-angular` still pins `webpack-dev-server` to the `5.2.x` line. This is accepted as a monitored risk — Dependabot runs weekly and will open a PR automatically once an upstream fix ships. Do not force an `npm overrides` entry for `uuid`, as that would deviate from `sockjs`'s own tested dependency contract for no production benefit.

## Additional Resources

For more information on using the Angular CLI, including detailed command references, visit the [Angular CLI Overview and Command Reference](https://angular.dev/tools/cli) page.
