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

## Internationalisation (i18n)

Every user-facing string of the interface lives in `public/i18n/<lang>.json` (`fr`, `en`, `es`, `it`,
`de`; `fr.json` is the reference language) and is read through [Transloco](https://jsverse.gitbook.io/transloco):

- In a template, use the pipe: `{{ 'auth.login.submit' | transloco }}`, or
  `[label]="'auth.login.username' | transloco"` for an attribute, with parameters as
  `{{ 'users.detail.email' | transloco: { email } }}` (`"E-mail : {{ email }}"` in the JSON).
- In TypeScript, call `t('some.key', { param })` from `shared/i18n/translator`. It works without an
  injection context (validators, formatters, error mappers). Do not call it at module level: the
  dictionary is only loaded when the application starts.
- Plurals are a `_one` / `_other` pair (`format.results_one`, `format.results_other`); the code picks
  the right key.
- `meta.locale` holds the BCP 47 tag used for number and relative-date formatting.

The language is picked at startup from the browser's `navigator.languages` (`fr-CA` gives `fr`),
English when none of them is translated (`shared/i18n/languages.ts`). Adding a language means a
new `<lang>.json`, an entry in `LANGUAGE_LOCALES` and its Angular locale data in `app.config.ts`.

The unit tests load the real `fr.json`, so they still assert the text users see. A spec
(`shared/i18n/translations.spec.ts`) fails when the code uses a key that `fr.json` does not define,
or when a key of `fr.json` is used nowhere.

## Known issues

### npm audit — devDependency-only UUID vulnerability (B-44)

`npm audit` reports 5 moderate findings in a devDependency chain (`uuid <11.1.1` via `sockjs` → `webpack-dev-server` → `@angular-devkit/build-angular`). These never reach production (`npm audit --omit=dev` reports 0 findings). No fix is currently available upstream: `sockjs` has not released a version compatible with `uuid ≥ 11.1.1`, and `@angular-devkit/build-angular` still pins `webpack-dev-server` to the `5.2.x` line. This is accepted as a monitored risk — Dependabot runs weekly and will open a PR automatically once an upstream fix ships. Do not force an `npm overrides` entry for `uuid`, as that would deviate from `sockjs`'s own tested dependency contract for no production benefit.

## Additional Resources

For more information on using the Angular CLI, including detailed command references, visit the [Angular CLI Overview and Command Reference](https://angular.dev/tools/cli) page.
