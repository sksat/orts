/**
 * Utilities for extracting cfg conditions from rustdoc JSON item attributes
 * and rendering them as Markdown badges.
 *
 * Nightly rustdoc (1.97+) automatically emits `CfgTrace` attributes for items
 * behind `#[cfg(...)]` gates — no explicit `#[doc(cfg(...))]` or
 * `doc_auto_cfg` feature flag is needed.
 */

import type { Attribute } from "./types.js";

/** A parsed cfg condition extracted from rustdoc attrs. */
export interface CfgCondition {
  /** Human-readable label for display (e.g. "alloc", "x86_64"). */
  label: string;
  /** Badge type: "feature" for crate features, "platform" for target, "other" for anything else. */
  kind: "feature" | "platform" | "other";
  /** Whether this condition is negated (e.g. not(target_arch = "wasm32")). */
  negated: boolean;
}

// Matches NameValue, optionally preceded by Not(.
// Group 1: "Not(" if negated, undefined otherwise.
// Group 2: name (e.g. "feature", "target_arch").
// Group 3: value (e.g. "alloc", "wasm32").
const NAME_VALUE_RE = /(Not\()?\s*NameValue\s*\{\s*name:\s*"(\w+)",\s*value:\s*Some\("([^"]+)"\)/g;

/**
 * Extract cfg conditions from an item's attrs array.
 *
 * Parses `Attribute::Other` values containing `CfgTrace(...)` entries
 * produced by nightly rustdoc's automatic cfg tracing.
 */
export function extractCfgConditions(attrs: Attribute[]): CfgCondition[] {
  const conditions: CfgCondition[] = [];
  const seen = new Set<string>();

  for (const attr of attrs) {
    if (typeof attr === "string") continue;
    const other = (attr as Record<string, unknown>).other;
    if (typeof other !== "string") continue;
    if (!other.includes("CfgTrace")) continue;

    NAME_VALUE_RE.lastIndex = 0;
    for (const m of other.matchAll(NAME_VALUE_RE)) {
      const negated = m[1] != null;
      const name = m[2];
      const value = m[3];
      if (!name || !value) continue;
      const key = `${negated ? "!" : ""}${name}:${value}`;
      if (seen.has(key)) continue;
      seen.add(key);

      if (name === "feature") {
        conditions.push({ label: value, kind: "feature", negated });
      } else if (name === "target_arch" || name === "target_os") {
        conditions.push({ label: value, kind: "platform", negated });
      } else {
        conditions.push({ label: `${name} = "${value}"`, kind: "other", negated });
      }
    }
  }

  return conditions;
}

/**
 * Render cfg conditions as a Markdown badge line.
 * Returns an empty string if there are no conditions.
 */
export function renderCfgBadge(conditions: CfgCondition[]): string {
  if (conditions.length === 0) return "";

  const features = conditions.filter((c) => c.kind === "feature" && !c.negated);
  const platforms = conditions.filter((c) => c.kind === "platform");
  const others = conditions.filter((c) => c.kind === "other");

  const parts: string[] = [];

  if (features.length === 1) {
    const feat = features[0];
    if (feat) parts.push(`Available on crate feature \`${feat.label}\` only.`);
  } else if (features.length > 1) {
    const labels = features.map((f) => `\`${f.label}\``);
    const last = labels.pop();
    if (last) {
      parts.push(`Available on crate features ${labels.join(", ")} and ${last} only.`);
    }
  }

  if (platforms.length > 0) {
    for (const p of platforms) {
      const label = p.negated ? `non-\`${p.label}\`` : `\`${p.label}\``;
      parts.push(`Available on ${label} only.`);
    }
  }

  if (others.length > 0) {
    const labels = others.map((o) => {
      return o.negated ? `not \`${o.label}\`` : `\`${o.label}\``;
    });
    parts.push(`Available when ${labels.join(", ")}.`);
  }

  if (parts.length === 0) return "";
  return `> **${parts.join(" ")}**\n`;
}
