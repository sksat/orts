import { existsSync, statSync } from "node:fs";
import { dirname, posix, relative, resolve, sep } from "node:path";
import type { Link, Root } from "mdast";
import type { VFile } from "vfile";

const GITHUB_BASE = "https://github.com/sksat/orts";
const GITHUB_BRANCH = "main";
const TARGET_RE = /[/\\]ARCHITECTURE(?:\.ja)?\.md$/;

export default function remarkRewriteRootMdLinks() {
  return (tree: Root, file: VFile) => {
    const mdPath = (file.path as string | undefined) ?? (file.history?.[0] as string | undefined);
    if (!mdPath || !TARGET_RE.test(mdPath)) return;

    const mdDir = dirname(mdPath);
    walk(tree, mdDir);
  };
}

function walk(node: { type: string; url?: string; children?: unknown[] }, mdDir: string): void {
  if (node.type === "link") {
    const link = node as unknown as Link;
    const rewritten = rewriteUrl(link.url, mdDir);
    if (rewritten !== null) link.url = rewritten;
  }
  if (Array.isArray(node.children)) {
    for (const child of node.children) {
      walk(child as { type: string; url?: string; children?: unknown[] }, mdDir);
    }
  }
}

export function rewriteUrl(url: string, mdDir: string): string | null {
  if (/^[a-z][a-z0-9+.-]*:/i.test(url)) return null;
  if (url.startsWith("#") || url.startsWith("/")) return null;

  const splitIdx = url.search(/[?#]/);
  const pathPart = splitIdx >= 0 ? url.slice(0, splitIdx) : url;
  const suffix = splitIdx >= 0 ? url.slice(splitIdx) : "";
  if (pathPart === "") return null;

  const abs = resolve(mdDir, pathPart);
  if (!existsSync(abs)) return null;

  const rel = relative(mdDir, abs);
  if (rel === "" || rel.startsWith("..")) return null;

  const kind = statSync(abs).isDirectory() ? "tree" : "blob";
  const ghPath = rel.split(sep).join(posix.sep);
  return `${GITHUB_BASE}/${kind}/${GITHUB_BRANCH}/${ghPath}${suffix}`;
}
