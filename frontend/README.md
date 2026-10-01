# ArtiFerris frontend

Angular 22, standalone components and signals.

```bash
ng serve        # http://localhost:4200
ng build
ng test         # Vitest
ng lint
npm run storybook
```

## Translations

Every string of the interface lives in `public/i18n/<lang>.json` (`fr`, `en`, `es`, `it`, `de`; `fr.json` is the
reference) and is read through [Transloco](https://jsverse.gitbook.io/transloco).

- In a template: `{{ 'auth.login.submit' | transloco }}`, with parameters as
  `{{ 'users.detail.email' | transloco: { email } }}` (`"E-mail : {{ email }}"` in the JSON).
- In TypeScript: `t('some.key', { param })` from `shared/i18n/translator`. It works without an injection context
  (validators, formatters, error mappers). Do not call it at module level: the dictionary loads when the app starts.
- Plurals are a `_one` / `_other` pair; `meta.locale` holds the BCP 47 tag used for numbers and relative dates.
- Adding a language means a new `<lang>.json`, an entry in `LANGUAGE_LOCALES` (`shared/i18n/languages.ts`) and its
  Angular locale data in `app.config.ts`.

**Which language wins**: the account's, then the browser's (`navigator.languages`, `fr-CA` gives `fr`), then English.
Signed out, the browser's applies. An account that has not chosen (`language: null`) takes the browser's at its first
sign-in and records it (`MeService`); if recording fails, the next sign-in tries again. The user changes it on the
account page; it is saved with `PUT /api/me/language` and read back from `GET /api/me`.

**Changing language while the app runs** (`LanguageService.use('de')`) loads the dictionary, then updates
`activeLanguage` (a signal) and `<html lang>`. A `computed` that calls `t()` follows by itself; the routed view is
re-created so labels built once (table columns, option lists) are rebuilt. Format dates with `formatLocalizedDate` or
the `date` pipe of `shared/i18n/localized-date.ts` and read the locale with `activeLocale()`, not Angular's `DatePipe`
or `LOCALE_ID`, which stay on the start-up locale.

Unit tests load the real `fr.json`. `shared/i18n/translations.spec.ts` fails when the code uses a key `fr.json` does
not define, or when a key is used nowhere.

## API errors

The API answers errors as `{ "error": "<English text>", "code": "<snake_case_name>" }`. The `code` is the contract
(each variant of `DomainError` and `ApplicationError` has one, see their `code()`); the text may be reworded. React to
`errorCode(err)` from `shared/api-error.ts`, never to a substring of the text. The user sees the translation
`errors.api.<code>`, and the server's text only for an error without a code. A new code needs its `errors.api.<code>`
message in every language file: `translations.spec.ts` reads the Rust sources and fails otherwise.
