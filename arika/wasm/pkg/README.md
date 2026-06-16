# arika-wasm

WebAssembly bindings for the [`arika`](../) coordinate/epoch/ephemeris library:
ECI↔ECEF transforms, sun direction, IAU body rotation, and attitude-frame
helpers. Built from the `arika-wasm` Rust crate (`../`) with
`wasm-pack --target web`.

This directory is a workspace package so consumers (the viewer, its embeddable
library, and the registry-distributed source) import it by name
(`import("arika-wasm")`) rather than by a relative path into another package's
source tree.

## Why `package.json` is committed but the build is not

`package.json`, `arika.d.ts`, and this README are committed so the workspace
resolves the package and TypeScript type-checks without a wasm build. The
runtime artifacts (`arika.js`, `arika_bg.wasm`, …) are git-ignored and
generated:

```sh
pnpm --filter orts-viewer build:wasm:arika
```

`build:wasm:arika` runs `wasm-pack` into a throwaway `../pkg-build` directory
(which `wasm-pack` fully manages — it rewrites `package.json` and a `.gitignore`
there) and copies only the build outputs into this curated package. That keeps
this hand-maintained `package.json` and the workspace layout under our control.
