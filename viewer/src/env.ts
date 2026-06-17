/**
 * Bundler-neutral access to the Vite `import.meta.env` values the viewer reads.
 *
 * This source is distributed via the shadcn registry to consumers that may not
 * use Vite (e.g. Next.js / Webpack), where `import.meta.env` is `undefined` —
 * and where TypeScript, lacking `vite/client` types, would reject
 * `import.meta.env` outright. Reading it once here, behind a cast and optional
 * chaining, keeps every other file portable: under Vite these are the real
 * values; elsewhere they fall back (base path "/", dev off).
 */
const env = (import.meta as unknown as { env?: { BASE_URL?: string; DEV?: boolean } }).env;

/** Base URL the app is served from (Vite `import.meta.env.BASE_URL`); "/" elsewhere. */
export const BASE_URL: string = env?.BASE_URL ?? "/";

/** Whether this is a development build (Vite `import.meta.env.DEV`); false elsewhere. */
export const IS_DEV: boolean = env?.DEV ?? false;
