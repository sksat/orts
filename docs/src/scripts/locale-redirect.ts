/**
 * Locale redirect logic for the orts documentation site.
 *
 * The documentation site is a Starlight + Astro project that publishes every
 * page under a locale segment (e.g. /orts/en/foo/ or /orts/ja/foo/). Some URL
 * shapes, however, do not naturally carry a locale:
 *
 * - The root URL `/orts/` (landing page)
 * - Any URL a visitor types without a locale prefix (e.g. /orts/getting-started/,
 *   which results in a GitHub Pages 404)
 * - Links from outside the site that predate the i18n migration
 *
 * For these URLs we want to redirect to the locale the visitor is most likely
 * to read, following this priority:
 *
 *   1. An explicit user choice previously saved to localStorage (set when
 *      Starlight's language picker navigates to a `/orts/<locale>/` URL —
 *      see {@link buildPersistCurrentLocaleScript}).
 *   2. The first supported locale found in `navigator.languages`, matching
 *      on the BCP 47 primary subtag so that e.g. `ja-JP` matches `ja`.
 *   3. A configured fallback locale (typically the Starlight defaultLocale).
 *
 * This file is imported from:
 *
 * - Astro pages `src/pages/index.astro` and `src/pages/404.astro`, which
 *   embed the runtime detection script via {@link buildInlineRedirectScript}
 *   and `<script is:inline set:html>`. The serialized script re-executes
 *   {@link chooseRedirectTarget} at page load using browser state.
 * - `astro.config.mjs`, which embeds {@link buildPersistCurrentLocaleScript}
 *   into the Starlight `head` config so every docs page records the
 *   visitor's current locale after any in-site navigation.
 * - `locale-redirect.test.ts`, which unit-tests {@link chooseRedirectTarget}
 *   directly. Because the runtime script is produced by serialising the
 *   very same function via `Function.prototype.toString`, the unit tests
 *   are a faithful check of the deployed behaviour.
 */

export interface RedirectInputs {
  /** URL base path including trailing slash (e.g. "/orts/"). */
  base: string;
  /** Current page path (e.g. from window.location.pathname). */
  path: string;
  /** Supported locale codes in preference order (e.g. ["en", "ja"]). */
  supported: readonly string[];
  /** Locale to use when nothing else matches — usually Starlight's defaultLocale. */
  fallback: string;
  /** Languages from navigator.languages (may contain BCP 47 subtags). */
  languages: readonly string[];
  /** Optional manual locale choice previously saved to localStorage. */
  storedLocale?: string | null;
  /** window.location.search (query string including leading "?"). */
  search?: string;
  /** window.location.hash (fragment including leading "#"). */
  hash?: string;
}

/**
 * Decide where a visitor should be redirected to land on a locale-prefixed
 * URL, or return `null` if the current URL is already locale-prefixed or
 * falls outside `base` (in which case the caller should leave the URL
 * alone — it is a real 404 or not one of our pages).
 *
 * This function is pure and deterministic so it can be unit-tested and
 * serialized into an inline browser script without drift between the two
 * surfaces (see {@link buildInlineRedirectScript}).
 */
export function chooseRedirectTarget(inputs: RedirectInputs): string | null {
  const {
    base,
    path,
    supported,
    fallback,
    languages,
    storedLocale,
    search = "",
    hash = "",
  } = inputs;

  if (!path.startsWith(base)) {
    return null;
  }
  const rest = path.slice(base.length);

  // Already locale-prefixed → no redirect (real 404 or existing page).
  for (const locale of supported) {
    if (rest === locale || rest === `${locale}/` || rest.startsWith(`${locale}/`)) {
      return null;
    }
  }

  // 1. Explicit user choice (localStorage)
  let chosen: string = fallback;
  if (storedLocale && supported.includes(storedLocale)) {
    chosen = storedLocale;
  } else {
    // 2. Browser preference via BCP 47 primary subtag
    for (const lang of languages) {
      const primary = (lang || "").split("-")[0].toLowerCase();
      if (supported.includes(primary)) {
        chosen = primary;
        break;
      }
    }
    // 3. (implicit) fallback
  }

  return `${base}${chosen}/${rest}${search}${hash}`;
}

/**
 * Build the inline browser script that gets embedded in `<head>` of
 * `src/pages/index.astro` and `src/pages/404.astro`.
 *
 * The script is produced by serializing {@link chooseRedirectTarget} via
 * `Function.prototype.toString()` so that there is **one** source of truth
 * for the redirect logic shared between unit tests and the runtime.
 */
export function buildInlineRedirectScript(config: {
  base: string;
  supported: readonly string[];
  fallback: string;
  /** localStorage key used to persist the visitor's manual locale choice. */
  storageKey: string;
}): string {
  const fnSource = chooseRedirectTarget.toString();
  const configJson = {
    base: config.base,
    supported: [...config.supported],
    fallback: config.fallback,
    storageKey: config.storageKey,
  };
  // IIFE — read browser state, compute the target, and navigate. Wrapped in
  // try/catch around localStorage access so that privacy-mode browsers (which
  // throw on getItem) degrade gracefully to the navigator.languages path.
  return `
(function () {
  var chooseRedirectTarget = ${fnSource};
  var cfg = ${JSON.stringify(configJson)};
  var stored = null;
  try { stored = window.localStorage.getItem(cfg.storageKey); } catch (e) {}
  var target = chooseRedirectTarget({
    base: cfg.base,
    path: window.location.pathname,
    supported: cfg.supported,
    fallback: cfg.fallback,
    languages: navigator.languages || [navigator.language || ""],
    storedLocale: stored,
    search: window.location.search,
    hash: window.location.hash,
  });
  if (target) window.location.replace(target);
})();
`.trim();
}

/**
 * Build the inline script that runs on every Starlight-rendered docs page
 * and records the visitor's current locale to localStorage.
 *
 * How it works: whenever a page loads (including after the visitor changes
 * language via Starlight's built-in picker, which navigates to the other
 * locale's URL), we parse the URL for its locale segment and save it. On
 * the next visit to a locale-less URL (e.g. the root or a stale external
 * link), {@link chooseRedirectTarget} finds the stored value and honours
 * it over `navigator.languages`.
 */
export function buildPersistCurrentLocaleScript(config: {
  base: string;
  supported: readonly string[];
  storageKey: string;
}): string {
  const configJson = {
    base: config.base,
    supported: [...config.supported],
    storageKey: config.storageKey,
  };
  return `
(function () {
  var cfg = ${JSON.stringify(configJson)};
  var path = window.location.pathname;
  if (path.indexOf(cfg.base) !== 0) return;
  var rest = path.slice(cfg.base.length);
  var segment = rest.split("/")[0];
  if (cfg.supported.indexOf(segment) === -1) return;
  try { window.localStorage.setItem(cfg.storageKey, segment); } catch (e) {}
})();
`.trim();
}

/**
 * Parse the locale segment of a URL under `base`. Returns the locale code
 * if it matches `supported`, otherwise null. Exported for unit tests that
 * mirror the logic of {@link buildPersistCurrentLocaleScript} without
 * having to spin up a DOM.
 */
export function detectLocaleFromPath(opts: {
  base: string;
  path: string;
  supported: readonly string[];
}): string | null {
  const { base, path, supported } = opts;
  if (!path.startsWith(base)) return null;
  const rest = path.slice(base.length);
  const segment = rest.split("/")[0];
  return supported.includes(segment) ? segment : null;
}
