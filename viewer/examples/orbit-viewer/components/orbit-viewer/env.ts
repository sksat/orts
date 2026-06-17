const env = (import.meta as unknown as { env?: { BASE_URL?: string; DEV?: boolean } }).env;

/** Base URL the app is served from (Vite `import.meta.env.BASE_URL`); "/" elsewhere. */
export const BASE_URL: string = env?.BASE_URL ?? "/";

/** Whether this is a development build (Vite `import.meta.env.DEV`); false elsewhere. */
export const IS_DEV: boolean = env?.DEV ?? false;
