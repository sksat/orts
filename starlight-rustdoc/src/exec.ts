import { execSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import type { Crate } from "./types.js";

export interface CrateSpec {
  name: string;
  features?: string[];
  noDefaultFeatures?: boolean;
  allFeatures?: boolean;
}

export function normalizeCrateSpec(crate: string | CrateSpec): CrateSpec {
  return typeof crate === "string" ? { name: crate } : crate;
}

export function generateRustdocJson(
  crate: CrateSpec,
  options: {
    workspace: string;
    toolchain?: string;
  },
): Crate {
  const toolchain = options.toolchain ?? "nightly";

  const args = [`cargo`, `+${toolchain}`, `rustdoc`, `-p`, crate.name];

  if (crate.noDefaultFeatures) {
    args.push("--no-default-features");
  }
  if (crate.allFeatures) {
    args.push("--all-features");
  } else if (crate.features && crate.features.length > 0) {
    args.push("--features", crate.features.join(","));
  }

  // Flags after -- are passed to rustdoc itself
  args.push("--", "--output-format", "json", "-Z", "unstable-options");

  execSync(args.join(" "), {
    cwd: options.workspace,
    stdio: "pipe",
  });

  // rustdoc outputs to target/doc/{crate_name}.json
  // Crate names with hyphens are converted to underscores in the filename
  const filename = crate.name.replace(/-/g, "_");
  const jsonPath = resolve(options.workspace, "target", "doc", `${filename}.json`);
  const raw = readFileSync(jsonPath, "utf-8");
  return JSON.parse(raw) as Crate;
}
