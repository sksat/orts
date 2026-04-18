#!/usr/bin/env bash
#
# check-versions.sh — Verify that workspace crate and npm package versions
# are consistent across the orts monorepo.
#
# Exit 0 if everything matches, exit 1 on any mismatch.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ERRORS=0

pass() { printf "  \033[32mPASS\033[0m %s\n" "$1"; }
fail() { printf "  \033[31mFAIL\033[0m %s\n" "$1"; ERRORS=$((ERRORS + 1)); }
info() { printf "\033[1m%s\033[0m\n" "$1"; }

# ---------------------------------------------------------------------------
# 1. Extract workspace version from root Cargo.toml
# ---------------------------------------------------------------------------
info "Checking workspace version..."

WORKSPACE_VERSION=$(python3 -c "
import re, sys
text = open('${REPO_ROOT}/Cargo.toml').read()
# Match [workspace.package] section's version field
m = re.search(r'\[workspace\.package\].*?^version\s*=\s*\"([^\"]+)\"', text, re.MULTILINE | re.DOTALL)
if not m:
    print('NOT_FOUND', end='')
    sys.exit(1)
print(m.group(1), end='')
")

if [ -z "$WORKSPACE_VERSION" ] || [ "$WORKSPACE_VERSION" = "NOT_FOUND" ]; then
    fail "Could not extract [workspace.package] version from root Cargo.toml"
    exit 1
fi

pass "Workspace version: ${WORKSPACE_VERSION}"

# ---------------------------------------------------------------------------
# 2. Check each workspace member Cargo.toml uses version.workspace = true
# ---------------------------------------------------------------------------
info "Checking Cargo workspace members inherit version..."

# Extract workspace members list from root Cargo.toml
MEMBERS=$(python3 -c "
import re
text = open('${REPO_ROOT}/Cargo.toml').read()
m = re.search(r'\[workspace\]\s*\n.*?members\s*=\s*\[(.*?)\]', text, re.DOTALL)
if m:
    for name in re.findall(r'\"([^\"]+)\"', m.group(1)):
        print(name)
")

for member in $MEMBERS; do
    cargo_toml="${REPO_ROOT}/${member}/Cargo.toml"
    if [ ! -f "$cargo_toml" ]; then
        fail "${member}/Cargo.toml not found"
        continue
    fi

    if grep -qE '^\s*version\.workspace\s*=\s*true' "$cargo_toml"; then
        pass "${member}/Cargo.toml uses version.workspace = true"
    elif grep -qE '^\s*version\s*=\s*"' "$cargo_toml"; then
        actual=$(grep -oP '^\s*version\s*=\s*"\K[^"]+' "$cargo_toml" | head -1)
        fail "${member}/Cargo.toml has hardcoded version \"${actual}\" instead of version.workspace = true"
    else
        fail "${member}/Cargo.toml has no version field"
    fi
done

# ---------------------------------------------------------------------------
# 3. Check [workspace.dependencies] internal crate versions match workspace
# ---------------------------------------------------------------------------
info "Checking [workspace.dependencies] internal crate versions..."

INTERNAL_CRATES="utsuroi arika orts tobari"
for crate in $INTERNAL_CRATES; do
    dep_version=$(python3 -c "
import re, sys
text = open('${REPO_ROOT}/Cargo.toml').read()
# Find the line for this crate in [workspace.dependencies]
pattern = r'^${crate}\s*=\s*\{[^}]*version\s*=\s*\"([^\"]+)\"'
m = re.search(pattern, text, re.MULTILINE)
if m:
    print(m.group(1), end='')
else:
    print('NOT_FOUND', end='')
")
    expected="=${WORKSPACE_VERSION}"
    if [ "$dep_version" = "$expected" ]; then
        pass "[workspace.dependencies] ${crate} version = \"${dep_version}\""
    elif [ "$dep_version" = "NOT_FOUND" ]; then
        fail "[workspace.dependencies] ${crate}: version field not found"
    else
        fail "[workspace.dependencies] ${crate} version = \"${dep_version}\" (expected \"${expected}\")"
    fi
done

# ---------------------------------------------------------------------------
# 4. Check npm package.json versions
# ---------------------------------------------------------------------------
info "Checking npm package versions..."

# Packages that must match the workspace version exactly
NPM_PACKAGES=(
    "uneri:${REPO_ROOT}/uneri/package.json"
    "orts-viewer:${REPO_ROOT}/viewer/package.json"
    "starlight-rustdoc:${REPO_ROOT}/starlight-rustdoc/package.json"
)

for entry in "${NPM_PACKAGES[@]}"; do
    name="${entry%%:*}"
    pkg_json="${entry#*:}"
    if [ ! -f "$pkg_json" ]; then
        fail "${name}: ${pkg_json} not found"
        continue
    fi
    pkg_version=$(python3 -c "
import json, sys
data = json.load(open('${pkg_json}'))
print(data.get('version', 'NOT_FOUND'), end='')
")
    if [ "$pkg_version" = "$WORKSPACE_VERSION" ]; then
        pass "${name} package.json version = \"${pkg_version}\""
    else
        fail "${name} package.json version = \"${pkg_version}\" (expected \"${WORKSPACE_VERSION}\")"
    fi
done

# tobari-example-web is private and may lag — warn but don't fail
TOBARI_WEB="${REPO_ROOT}/tobari/examples/web/package.json"
if [ -f "$TOBARI_WEB" ]; then
    tw_version=$(python3 -c "
import json
data = json.load(open('${TOBARI_WEB}'))
print(data.get('version', 'NOT_FOUND'), end='')
")
    if [ "$tw_version" = "$WORKSPACE_VERSION" ]; then
        pass "tobari-example-web package.json version = \"${tw_version}\""
    else
        printf "  \033[33mWARN\033[0m tobari-example-web package.json version = \"%s\" (workspace is \"%s\", private package — allowed to lag)\n" "$tw_version" "$WORKSPACE_VERSION"
    fi
fi

# ---------------------------------------------------------------------------
# 5. Check plugin-sdk/examples Cargo.lock references to orts-plugin-sdk
# ---------------------------------------------------------------------------
info "Checking plugin-sdk/examples Cargo.lock orts-plugin-sdk version..."

PLUGIN_LOCK="${REPO_ROOT}/plugin-sdk/examples/Cargo.lock"
if [ -f "$PLUGIN_LOCK" ]; then
    sdk_version=$(grep -A1 'name = "orts-plugin-sdk"' "$PLUGIN_LOCK" 2>/dev/null | grep 'version' | sed 's/.*"\(.*\)"/\1/' | head -1 || true)
    if [ -z "$sdk_version" ]; then
        pass "plugin-sdk/examples Cargo.lock does not reference orts-plugin-sdk (path dependency)"
    elif [ "$sdk_version" = "$WORKSPACE_VERSION" ]; then
        pass "plugin-sdk/examples Cargo.lock orts-plugin-sdk = \"${sdk_version}\""
    else
        fail "plugin-sdk/examples Cargo.lock orts-plugin-sdk = \"${sdk_version}\" (expected \"${WORKSPACE_VERSION}\")"
    fi
else
    fail "plugin-sdk/examples/Cargo.lock not found"
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
if [ "$ERRORS" -eq 0 ]; then
    printf "\033[32mAll version checks passed.\033[0m\n"
    exit 0
else
    printf "\033[31m%d version check(s) failed.\033[0m\n" "$ERRORS"
    exit 1
fi
