# orbit-viewer example — a shadcn registry consumer

A standalone, backend-free app that renders [`<OrbitViewer>`](../../README.md)
to **dogfood the viewer's shadcn registry**: instead of importing
`viewer/src/lib`, it consumes the registry's `orbit-viewer` item via
`shadcn add` and imports the copied source from
[`components/orbit-viewer/`](./components/orbit-viewer). It serves itself on its
own Vite and is exercised by [`tests/`](./tests) (also run in CI), so the
registry output is verified end to end — a real consumer installs it and it
compiles + runs.

## The copied source is committed

`components/orbit-viewer/` is the `shadcn add` output, committed like a real
consumer owns its copied components (note `shadcn add` strips each file's
leading banner comment — the registry source under `viewer/src` keeps them).
`arika-wasm` stays a workspace dependency (it isn't copied — see the registry
docs).

Regenerate it after the registry's source changes:

```sh
pnpm --filter orts-viewer-orbit-example sync:registry   # registry:build + shadcn add --overwrite
```

`sync:registry` runs `shadcn add` locally; CI does not (it would hit the
workspace's supply-chain release-age gate during dependency install), so the
copied tree is committed and CI runs the E2E against it.
