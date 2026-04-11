import { describe, expect, it } from "vitest";
import {
  buildInlineRedirectScript,
  buildPersistCurrentLocaleScript,
  chooseRedirectTarget,
  detectLocaleFromPath,
} from "./locale-redirect";

const CONFIG = {
  base: "/orts/",
  supported: ["en", "ja"] as const,
  fallback: "en",
  storageKey: "orts-locale",
};

// ---------------------------------------------------------------------------
// chooseRedirectTarget — browser language → locale
// ---------------------------------------------------------------------------

describe("chooseRedirectTarget — browser language detection", () => {
  it("redirects /orts/ to /orts/ja/ when the browser prefers Japanese", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/",
        languages: ["ja-JP", "ja", "en"],
      }),
    ).toBe("/orts/ja/");
  });

  it("redirects /orts/ to /orts/en/ when the browser prefers English", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/",
        languages: ["en-US", "en"],
      }),
    ).toBe("/orts/en/");
  });

  it("matches the BCP 47 primary subtag for Japanese regional variants", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/",
        languages: ["ja-JP"],
      }),
    ).toBe("/orts/ja/");
  });

  it("walks the languages array and picks the first supported locale", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/",
        languages: ["fr-FR", "de-DE", "ja", "en"],
      }),
    ).toBe("/orts/ja/");
  });

  it("falls back to the configured default locale for unsupported languages", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/",
        languages: ["fr-FR", "de-DE"],
      }),
    ).toBe("/orts/en/");
  });

  it("falls back when navigator.languages is empty", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/",
        languages: [],
      }),
    ).toBe("/orts/en/");
  });
});

// ---------------------------------------------------------------------------
// chooseRedirectTarget — nested paths
// ---------------------------------------------------------------------------

describe("chooseRedirectTarget — nested paths and query/hash", () => {
  it("prepends the locale to nested URLs without a locale segment", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/getting-started/",
        languages: ["ja-JP"],
      }),
    ).toBe("/orts/ja/getting-started/");
  });

  it("handles deep paths without trailing slash", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/arika/api/structs/epoch",
        languages: ["en"],
      }),
    ).toBe("/orts/en/arika/api/structs/epoch");
  });

  it("preserves query string and hash when redirecting", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/getting-started/",
        languages: ["ja"],
        search: "?foo=bar",
        hash: "#install",
      }),
    ).toBe("/orts/ja/getting-started/?foo=bar#install");
  });
});

// ---------------------------------------------------------------------------
// chooseRedirectTarget — already-locale-prefixed URLs
// ---------------------------------------------------------------------------

describe("chooseRedirectTarget — already-prefixed URLs pass through untouched", () => {
  it("returns null for /orts/en/ (root of default locale)", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/en/",
        languages: ["ja"],
      }),
    ).toBeNull();
  });

  it("returns null for /orts/ja/ (root of non-default locale)", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/ja/",
        languages: ["en"],
      }),
    ).toBeNull();
  });

  it("returns null for locale-prefixed nested URLs", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/en/getting-started/",
        languages: ["ja"],
      }),
    ).toBeNull();
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/ja/arika/api/structs/epoch/",
        languages: ["en"],
      }),
    ).toBeNull();
  });

  it("returns null when the path does not live under the base", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/other/site/",
        languages: ["en"],
      }),
    ).toBeNull();
  });

  it("does not redirect /en/... 404s caused by JA-only pages", () => {
    // Scenario: the author writes a new page in Japanese first without an
    // English counterpart (src/content/docs/ja/new-feature.mdx exists,
    // src/content/docs/en/new-feature.mdx does not). A visitor who lands on
    // /orts/en/new-feature/ gets a real 404. Our redirect script must NOT
    // bounce them to /orts/ja/... blindly — the URL already carries a
    // locale segment and Starlight's built-in 404 page (which contains the
    // language picker) is the correct surface. The visitor can then click
    // 日本語 in the picker to read the JA version.
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/en/new-feature/",
        languages: ["en"],
      }),
    ).toBeNull();
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/en/new-feature/",
        languages: ["ja"],
      }),
    ).toBeNull();
  });

  it("does not redirect /ja/... 404s either (symmetry)", () => {
    // Mirror of the previous case: a Japanese browser landing on a missing
    // /ja/... page still sees a real 404 rather than being bounced into
    // /en/...; avoids infinite redirect loops and respects the explicit
    // locale segment the visitor requested.
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/ja/never-existed/",
        languages: ["ja"],
      }),
    ).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// chooseRedirectTarget — localStorage persistence
// ---------------------------------------------------------------------------

describe("chooseRedirectTarget — localStorage override", () => {
  it("honours a stored ja choice over an English browser", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/",
        languages: ["en-US", "en"],
        storedLocale: "ja",
      }),
    ).toBe("/orts/ja/");
  });

  it("honours a stored en choice over a Japanese browser", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/",
        languages: ["ja-JP", "ja"],
        storedLocale: "en",
      }),
    ).toBe("/orts/en/");
  });

  it("honours stored choice for nested paths too", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/getting-started/",
        languages: ["en"],
        storedLocale: "ja",
      }),
    ).toBe("/orts/ja/getting-started/");
  });

  it("ignores an unsupported stored locale and falls through to browser", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/",
        languages: ["ja"],
        storedLocale: "fr",
      }),
    ).toBe("/orts/ja/");
  });

  it("ignores an empty string stored locale", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/",
        languages: ["ja"],
        storedLocale: "",
      }),
    ).toBe("/orts/ja/");
  });

  it("ignores a null stored locale", () => {
    expect(
      chooseRedirectTarget({
        ...CONFIG,
        path: "/orts/",
        languages: ["ja"],
        storedLocale: null,
      }),
    ).toBe("/orts/ja/");
  });
});

// ---------------------------------------------------------------------------
// detectLocaleFromPath — companion to the persist script
// ---------------------------------------------------------------------------

describe("detectLocaleFromPath", () => {
  it("extracts the locale segment from a locale-prefixed URL", () => {
    expect(
      detectLocaleFromPath({
        base: "/orts/",
        path: "/orts/en/getting-started/",
        supported: ["en", "ja"],
      }),
    ).toBe("en");
    expect(
      detectLocaleFromPath({
        base: "/orts/",
        path: "/orts/ja/arika/api/overview/",
        supported: ["en", "ja"],
      }),
    ).toBe("ja");
  });

  it("returns null for URLs without a locale prefix", () => {
    expect(
      detectLocaleFromPath({
        base: "/orts/",
        path: "/orts/getting-started/",
        supported: ["en", "ja"],
      }),
    ).toBeNull();
    expect(
      detectLocaleFromPath({
        base: "/orts/",
        path: "/orts/",
        supported: ["en", "ja"],
      }),
    ).toBeNull();
  });

  it("returns null for URLs outside the base", () => {
    expect(
      detectLocaleFromPath({
        base: "/orts/",
        path: "/other/en/",
        supported: ["en", "ja"],
      }),
    ).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// buildInlineRedirectScript — runtime script integrity
// ---------------------------------------------------------------------------

describe("buildInlineRedirectScript", () => {
  const script = buildInlineRedirectScript(CONFIG);

  it("serializes chooseRedirectTarget into the script body", () => {
    // If the pure function name is missing, the runtime script would be
    // referencing a phantom identifier — this catches refactors that
    // break serialization via Function.prototype.toString.
    expect(script).toContain("chooseRedirectTarget");
    // Key behaviours must appear in the emitted source.
    expect(script).toContain("storedLocale");
    expect(script).toContain("navigator.languages");
    expect(script).toContain("localStorage");
    expect(script).toContain("window.location.replace");
  });

  it("embeds the configuration JSON verbatim", () => {
    expect(script).toContain('"base":"/orts/"');
    expect(script).toContain('"supported":["en","ja"]');
    expect(script).toContain('"fallback":"en"');
    expect(script).toContain('"storageKey":"orts-locale"');
  });

  it("runs without errors in a mocked window and performs the correct redirect", () => {
    // Evaluate the script against a manually-constructed mock window so we
    // can prove that the script AS DEPLOYED (not just the pure function)
    // produces the expected side effect.
    const replaced: string[] = [];
    const fakeWindow = {
      location: {
        pathname: "/orts/getting-started/",
        search: "",
        hash: "",
        replace: (url: string) => {
          replaced.push(url);
        },
      },
      localStorage: {
        getItem: (_key: string) => "ja",
        setItem: () => {},
      },
    };
    const fakeNavigator = {
      languages: ["en"],
      language: "en",
    };
    // Provide `window` and `navigator` as globals, since the serialized
    // IIFE accesses them unqualified.
    new Function("window", "navigator", script)(fakeWindow, fakeNavigator);
    // Stored "ja" should win over the English browser language.
    expect(replaced).toEqual(["/orts/ja/getting-started/"]);
  });

  it("falls back to navigator.languages when localStorage is empty", () => {
    const replaced: string[] = [];
    const fakeWindow = {
      location: {
        pathname: "/orts/",
        search: "",
        hash: "",
        replace: (url: string) => {
          replaced.push(url);
        },
      },
      localStorage: {
        getItem: (_key: string) => null,
        setItem: () => {},
      },
    };
    const fakeNavigator = {
      languages: ["ja-JP", "ja"],
      language: "ja-JP",
    };
    new Function("window", "navigator", script)(fakeWindow, fakeNavigator);
    expect(replaced).toEqual(["/orts/ja/"]);
  });

  it("does not redirect when the URL is already locale-prefixed", () => {
    const replaced: string[] = [];
    const fakeWindow = {
      location: {
        pathname: "/orts/en/getting-started/",
        search: "",
        hash: "",
        replace: (url: string) => {
          replaced.push(url);
        },
      },
      localStorage: {
        getItem: (_key: string) => "ja",
        setItem: () => {},
      },
    };
    const fakeNavigator = {
      languages: ["ja"],
      language: "ja",
    };
    new Function("window", "navigator", script)(fakeWindow, fakeNavigator);
    expect(replaced).toEqual([]);
  });

  it("degrades gracefully when localStorage throws (private mode)", () => {
    const replaced: string[] = [];
    const fakeWindow = {
      location: {
        pathname: "/orts/",
        search: "",
        hash: "",
        replace: (url: string) => {
          replaced.push(url);
        },
      },
      localStorage: {
        getItem: () => {
          throw new Error("SecurityError: localStorage not available");
        },
        setItem: () => {},
      },
    };
    const fakeNavigator = {
      languages: ["ja"],
      language: "ja",
    };
    new Function("window", "navigator", script)(fakeWindow, fakeNavigator);
    // Should fall back to navigator.languages
    expect(replaced).toEqual(["/orts/ja/"]);
  });
});

// ---------------------------------------------------------------------------
// buildPersistCurrentLocaleScript — every-page locale-saving script
// ---------------------------------------------------------------------------

describe("buildPersistCurrentLocaleScript", () => {
  const script = buildPersistCurrentLocaleScript(CONFIG);

  it("embeds the configuration JSON", () => {
    expect(script).toContain('"base":"/orts/"');
    expect(script).toContain('"supported":["en","ja"]');
    expect(script).toContain('"storageKey":"orts-locale"');
  });

  it("writes the current locale to localStorage when on an en page", () => {
    const store = new Map<string, string>();
    const fakeWindow = {
      location: { pathname: "/orts/en/getting-started/" },
      localStorage: {
        getItem: (k: string) => store.get(k) ?? null,
        setItem: (k: string, v: string) => {
          store.set(k, v);
        },
      },
    };
    new Function("window", script)(fakeWindow);
    expect(store.get("orts-locale")).toBe("en");
  });

  it("writes the current locale to localStorage when on a ja page", () => {
    const store = new Map<string, string>();
    const fakeWindow = {
      location: { pathname: "/orts/ja/arika/api/overview/" },
      localStorage: {
        getItem: (k: string) => store.get(k) ?? null,
        setItem: (k: string, v: string) => {
          store.set(k, v);
        },
      },
    };
    new Function("window", script)(fakeWindow);
    expect(store.get("orts-locale")).toBe("ja");
  });

  it("overwrites an existing stored locale when the user switches language", () => {
    // Simulates the user clicking Starlight's language picker to jump from
    // /en/... to /ja/... — on the next page load the persist script should
    // update the stored preference.
    const store = new Map<string, string>([["orts-locale", "en"]]);
    const fakeWindow = {
      location: { pathname: "/orts/ja/getting-started/" },
      localStorage: {
        getItem: (k: string) => store.get(k) ?? null,
        setItem: (k: string, v: string) => {
          store.set(k, v);
        },
      },
    };
    new Function("window", script)(fakeWindow);
    expect(store.get("orts-locale")).toBe("ja");
  });

  it("does nothing when the URL is not locale-prefixed", () => {
    const store = new Map<string, string>();
    const fakeWindow = {
      location: { pathname: "/orts/getting-started/" },
      localStorage: {
        getItem: (k: string) => store.get(k) ?? null,
        setItem: (k: string, v: string) => {
          store.set(k, v);
        },
      },
    };
    new Function("window", script)(fakeWindow);
    expect(store.size).toBe(0);
  });

  it("does nothing when the URL is outside the base", () => {
    const store = new Map<string, string>();
    const fakeWindow = {
      location: { pathname: "/other/en/" },
      localStorage: {
        getItem: (k: string) => store.get(k) ?? null,
        setItem: (k: string, v: string) => {
          store.set(k, v);
        },
      },
    };
    new Function("window", script)(fakeWindow);
    expect(store.size).toBe(0);
  });

  it("degrades gracefully when localStorage throws (private mode)", () => {
    const fakeWindow = {
      location: { pathname: "/orts/en/" },
      localStorage: {
        getItem: () => {
          throw new Error("denied");
        },
        setItem: () => {
          throw new Error("denied");
        },
      },
    };
    // Should not throw to the caller.
    expect(() => {
      new Function("window", script)(fakeWindow);
    }).not.toThrow();
  });
});
