import react from "@astrojs/react";
import starlight from "@astrojs/starlight";
import { defineConfig } from "astro/config";
import rehypeKatex from "rehype-katex";
import remarkMath from "remark-math";
import starlightRustdoc from "starlight-rustdoc";
import starlightTypeDoc from "starlight-typedoc";
import rewriteTypedocLinks from "./src/integrations/rewrite-typedoc-links";
import {
  buildInlineRedirectScript,
  buildPersistCurrentLocaleScript,
} from "./src/scripts/locale-redirect";

// Shared locale configuration for both the persist-current-locale hook and
// the locale-less URL redirect hook. Keeping this in one place means the two
// scripts can never disagree on supported locales or storage keys.
const LOCALE_CONFIG = {
  base: "/orts/",
  supported: ["en", "ja"],
  fallback: "en",
  storageKey: "orts-locale",
};

// Inline script added to the <head> of every Starlight-rendered page. It
// reads the URL's locale segment and saves it to localStorage so that later
// visits to locale-less URLs honour the visitor's manual choice made via
// Starlight's language picker. See src/scripts/locale-redirect.ts.
const persistLocaleScript = buildPersistCurrentLocaleScript(LOCALE_CONFIG);

// Inline script added to the <head> of every Starlight-rendered page that
// runs `chooseRedirectTarget` at page load. It is a no-op on valid
// locale-prefixed URLs (every real Starlight page) and active on Starlight's
// built-in 404 page, which GitHub Pages serves for arbitrary missing URLs
// such as `/orts/getting-started/` (no locale segment). In that case the
// original requested pathname is still in `window.location`, so the script
// can redirect the visitor to the matching locale version of the path.
//
// We deliberately route through Starlight's built-in 404 rather than a
// custom `src/pages/404.astro` because Starlight already registers a static
// `/404` route — defining a second one causes Astro to warn and will become
// a hard error in future versions. Injecting the redirect behaviour via
// `head` config leaves Starlight's 404 route untouched while still giving us
// the locale-less URL handling we want.
const redirectScript = buildInlineRedirectScript(LOCALE_CONFIG);

// GA4 tag — injected only in production build when a Measurement ID is set.
// The ID is supplied at build time via `PUBLIC_GA_MEASUREMENT_ID` (GitHub
// Actions passes it from a repository variable — GA4 Measurement IDs are
// public by design, so a `vars.*` is used rather than a secret).
//
// `import.meta.env` is not populated in astro.config.mjs (it runs in plain
// Node before Vite wires that up), so we read from process.env directly.
// `astro build` sets NODE_ENV=production; `astro dev` sets it to development
// — so this block stays silent during `pnpm dev`.
// For local production preview:
//   PUBLIC_GA_MEASUREMENT_ID=G-LMW888TV62 pnpm build
const GA_MEASUREMENT_ID = process.env.PUBLIC_GA_MEASUREMENT_ID;
const gaHeadEntries =
  process.env.NODE_ENV === "production" && GA_MEASUREMENT_ID
    ? [
        {
          tag: "script",
          attrs: {
            async: true,
            src: `https://www.googletagmanager.com/gtag/js?id=${GA_MEASUREMENT_ID}`,
          },
        },
        {
          tag: "script",
          attrs: { "is:inline": true },
          content:
            `window.dataLayer = window.dataLayer || [];` +
            `function gtag(){dataLayer.push(arguments);}` +
            `gtag('js', new Date());` +
            `gtag('config', '${GA_MEASUREMENT_ID}');`,
        },
      ]
    : [];

export default defineConfig({
  base: "/orts",
  site: "https://sksat.github.io",
  markdown: {
    remarkPlugins: [remarkMath],
    rehypePlugins: [rehypeKatex],
  },
  // Root (/) is handled by src/pages/index.astro and 404 handling is done
  // via a script injected into Starlight's built-in 404 page through the
  // `head` config below — see the comment on `redirectScript` above for why
  // we do not ship a custom src/pages/404.astro.
  integrations: [
    react(),
    starlight({
      title: "orts",
      customCss: ["katex/dist/katex.min.css", "./src/styles/katex.css"],
      defaultLocale: "en",
      locales: {
        en: { label: "English", lang: "en" },
        ja: { label: "日本語", lang: "ja" },
      },
      head: [
        // Runs on every Starlight page: record the visitor's current locale
        // so the redirect script below (or src/pages/index.astro) honours
        // their manual choice on later visits.
        {
          tag: "script",
          attrs: { "is:inline": true },
          content: persistLocaleScript,
        },
        // No-op on every valid Starlight page (they all carry a locale
        // segment). Active when Starlight's built-in 404 is served by
        // GitHub Pages for a locale-less URL such as /orts/getting-started/
        // — in that case we detect the visitor's preferred locale and
        // redirect to /orts/<locale>/getting-started/.
        {
          tag: "script",
          attrs: { "is:inline": true },
          content: redirectScript,
        },
        ...gaHeadEntries,
      ],
      social: [{ icon: "github", label: "GitHub", href: "https://github.com/sksat/orts" }],
      plugins: [
        starlightTypeDoc({
          entryPoints: ["../uneri/src/index.ts"],
          tsconfig: "../uneri/tsconfig.json",
          output: "en/uneri/api",
        }),
        starlightRustdoc({
          crates: [
            { name: "orts", features: ["fetch-weather", "fetch-horizons"] },
            { name: "arika", allFeatures: true },
            "utsuroi",
            { name: "tobari", features: ["fetch"] },
          ],
          workspace: "..",
          locale: "en",
          sidebar: false,
          sourceLinks: {
            repository: "https://github.com/sksat/orts",
          },
        }),
      ],
      sidebar: [
        { label: "Getting Started", slug: "getting-started" },
        { label: "Examples", autogenerate: { directory: "examples" } },
        {
          label: "orts",
          collapsed: true,
          items: [
            { label: "Overview", slug: "orts/overview" },
            {
              label: "API",
              collapsed: true,
              autogenerate: { directory: "orts/api" },
            },
          ],
        },
        {
          label: "arika",
          collapsed: true,
          items: [
            { label: "Overview", slug: "arika/overview" },
            {
              label: "API",
              collapsed: true,
              autogenerate: { directory: "arika/api" },
            },
          ],
        },
        {
          label: "utsuroi",
          collapsed: true,
          items: [
            { label: "Overview", slug: "utsuroi/overview" },
            {
              label: "API",
              collapsed: true,
              autogenerate: { directory: "utsuroi/api" },
            },
          ],
        },
        {
          label: "tobari",
          collapsed: true,
          items: [
            { label: "Overview", slug: "tobari/overview" },
            { label: "Examples", autogenerate: { directory: "tobari/examples" } },
            {
              label: "API",
              collapsed: true,
              autogenerate: { directory: "tobari/api" },
            },
          ],
        },
        {
          label: "uneri",
          collapsed: true,
          items: [
            { label: "Overview", slug: "uneri/api/readme" },
            { label: "Examples", autogenerate: { directory: "uneri/examples" } },
            {
              label: "API Reference",
              collapsed: true,
              items: [
                { label: "Classes", autogenerate: { directory: "uneri/api/classes" } },
                { label: "Interfaces", autogenerate: { directory: "uneri/api/interfaces" } },
                { label: "Functions", autogenerate: { directory: "uneri/api/functions" } },
                { label: "Type Aliases", autogenerate: { directory: "uneri/api/type-aliases" } },
                { label: "Variables", autogenerate: { directory: "uneri/api/variables" } },
              ],
            },
          ],
        },
      ],
    }),
    // Must come AFTER starlight() so that starlight-typedoc (invoked via
    // Starlight's own config:setup hook) has already written its Markdown
    // files to disk by the time this integration walks them. See
    // src/integrations/rewrite-typedoc-links.ts for why this is necessary.
    rewriteTypedocLinks({
      locale: "en",
      typedocRoot: "uneri/api",
      base: "/orts",
    }),
  ],
});
