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

## Recommended install

```
cargo binstall --git https://github.com/sksat/orts orts-cli --version 0.1.0-beta.1
```

(Or `cargo install orts-cli` once published.)

