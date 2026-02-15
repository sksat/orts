---
name: check-pkg-name
description: Check if package names are available on crates.io and npm. Use when naming a new library or crate.
argument-hint: "name1 name2 name3 ..."
allowed-tools:
  - Bash
---

# Check Package Name Availability

Check whether package names are taken on crates.io and npm.

## Usage

The user's argument is a space-separated list of candidate names. For each name, check both registries.

## Steps

1. Parse the argument into a list of names (split on spaces)
2. For each name, run these two curl commands **in parallel across all names** (use a single bash loop):

```bash
for name in <names>; do
  crate_code=$(curl -s -o /dev/null -w "%{http_code}" "https://crates.io/api/v1/crates/$name")
  npm_code=$(curl -s -o /dev/null -w "%{http_code}" "https://registry.npmjs.org/$name")
  echo "$name crates.io=$crate_code npm=$npm_code"
done
```

3. For any name that returns HTTP 200 (taken), fetch the description:
   - crates.io: `curl -s "https://crates.io/api/v1/crates/$name" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['crate']['description'])"`
   - npm: `curl -s "https://registry.npmjs.org/$name" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('description','(no description)'))"`

4. Present results in a table:

| Name | crates.io | npm | Both free? |
|------|-----------|-----|------------|
| foo  | free      | free | Yes       |
| bar  | **taken** (description) | free | No |

## Guidelines

- Add `sleep 0.3` between crates.io requests to avoid rate limiting
- HTTP 404 = available, HTTP 200 = taken
- If a registry returns an unexpected status code, report it as-is
