# orts-cli

orts CLI — orbital mechanics simulator with an embedded 3D viewer and
WebSocket telemetry server.

Ships the `orts` binary, which drives the
[orts](https://github.com/sksat/orts) simulation engine and bundles the
React + Three.js viewer SPA as a single `cargo install`-able distributable.

## Usage

```
orts run --sat "altitude=400" --dt 5   # run a quick sim, record to RRD
orts serve                             # WebSocket server (port 9001) +
                                       # embedded viewer at http://localhost:9001
```

See `orts --help` for the full CLI surface.

## Logs

Diagnostics go to stderr; stdout carries only what a command produces (CSV, the
`--json` summary, the `serve --stream-stdio` protocol). `RUST_LOG` sets the
filter, defaulting to `warn,orts=info` — orts at info, dependencies at warn.

```
RUST_LOG=warn orts serve --config mission.toml    # warnings and errors only
RUST_LOG=orts=debug orts run --config mission.toml
```

A WASM plugin's own log output (the WIT `host-env.log` import) arrives under the
same `orts` target, so `RUST_LOG=warn` silences flight-software logs too.

## Recommended install

```
cargo binstall --git https://github.com/sksat/orts orts-cli --version 0.1.0-beta.1
```

(Or `cargo install orts-cli` once published.)

