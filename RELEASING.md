# Releasing orts

orts のリリース手順メモ。

## リリースの種類

1. **GitHub Release (バイナリ配布)** — `.github/workflows/ci.yml` が
   tag push で自動発火（専用の release.yml は存在しない。CI と release が
   同一ワークフロー）。`v*` tag を打つとフル CI (lint, test, build 等) が
   走ったあと、`rust-dist` job で release binary をビルドし、`release` job
   で draft release を作成。添付物:
   各ターゲット (gnu + musl) について:
   - `orts-cli-{version}-{target}.tar.gz` (binary + README + LICENSE + example WASM plugins)
   - `orts-cli-{version}-{target}.tar.gz.sha256`
   - `orts-{target}` (standalone stripped binary)
   - `README.md`, `LICENSE`

   ターゲット: `x86_64-unknown-linux-gnu` (glibc), `x86_64-unknown-linux-musl` (static)

   draft release なので、人間が確認してから手動で publish する。
   cargo-binstall compatible。
2. **crates.io publish** — Rust library の公開。
   手動 publish。
3. **npm publish** — `uneri` + `starlight-rustdoc`。
   手動 publish。

## Pre-release checklist

main branch で clean な状態から:

```sh
# 1. 最新を pull
git checkout main
git pull --ff-only

# 2. Rust pre-commit gates
cargo fmt --all -- --check
cargo clippy --workspace --locked -- -D warnings
cargo clippy --locked -p tobari --features wasm -- -D warnings
cargo test --workspace --locked

# 3. TypeScript pre-commit gates
pnpm lint
pnpm --filter uneri build
pnpm --filter uneri test
pnpm --filter orts-viewer build
pnpm --filter orts-viewer test
pnpm --filter starlight-rustdoc build
pnpm --filter starlight-rustdoc test
pnpm --filter tobari-example-web build

# 4. Full site build (docs)
pnpm build:site
```

## バージョン bump

編集対象:

- `Cargo.toml` `[workspace.package] version` — 全 Rust crate に伝播
- `Cargo.toml` `[workspace.dependencies]` の内部 path dep の `version` —
  workspace version と同期させる (`utsuroi` / `arika` / `orts` / `tobari`)
- `plugin-sdk/Cargo.toml` — workspace inheritance 経由で自動
- `viewer/package.json` `version`
- `uneri/package.json` `version`
- `starlight-rustdoc/package.json` `version`

手順:

```sh
# Edit files above, then:
cargo check --workspace               # Cargo.lock 更新
cargo check --workspace --locked      # 再検証

# Tests (full suite)
cargo test --workspace --locked

# Commit
git add Cargo.toml Cargo.lock viewer/package.json uneri/package.json \
        starlight-rustdoc/package.json
git commit -m "chore: bump workspace to X.Y.Z"

# Push & wait CI green
git push origin main
gh run watch
```

## Tag push → GitHub Release (バイナリ配布)

`v*` tag push で `.github/workflows/ci.yml` のフルパイプラインが走る。
通常の CI job (lint, rust-test, viewer-build 等) に加えて:

1. **`rust-dist` job** (matrix: gnu + musl, needs: rust-build, viewer-build) —
   `--release` で CLI binary をビルド (viewer SPA を embed)
2. **`build-example-plugins` job** — WASM plugin guest を全自動ビルド
3. **`release` job** (needs: rust-dist, build-example-plugins, crate-package-verify,
   npm-publish-dry-run, rust-test, rust-test-plugin-wasm, viewer-e2e,
   cli-plugin-backend-e2e) — 全テスト + 全検証が通った場合のみ tarball +
   checksum を作成し `softprops/action-gh-release` で **draft** GitHub Release を作成

ターゲット: `x86_64-unknown-linux-gnu` (glibc) + `x86_64-unknown-linux-musl` (static)。

### Release の添付物 (各ターゲットについて)

- `orts-cli-{version}-{target}.tar.gz` — binary + README + LICENSE + `examples/plugins/*.wasm`
- `orts-cli-{version}-{target}.tar.gz.sha256`
- `orts-{target}` — standalone stripped binary (tarball 不要で直接使いたい場合)
- `README.md`, `LICENSE`

### 手順

```sh
# main が green の状態で tag
git tag -a v0.1.0 -m "orts 0.1.0"
git push origin v0.1.0

# ci.yml が発火、フル CI + rust-dist + release job を実行
gh run watch

# Draft release の確認 & 公開
gh release view v0.1.0
# 問題なければ draft を publish
gh release edit v0.1.0 --draft=false
```

### 失敗時の対応

- **Version mismatch**: tag 名と `Cargo.toml` version が不一致 →
  release job 冒頭の verify step で早期 fail。version bump 忘れ。
- **cargo-about が無い**: `cli/build.rs` が panic。CI runner 側で
  `taiki-e/install-action` で install されるはず。
- **viewer build 失敗**: viewer-build job で fail、rust-dist は実行されない。
  local で `pnpm --filter orts-viewer build` を再現。
- **example plugin build 失敗**: rust-dist job の cargo-component build step
  で fail。local で `(cd plugins/xxx && cargo component build --release)` を再現。

## crates.io publish

crate 間の依存関係があるため、以下の順序で publish する必要がある。
**同じ tier 内は並列 OK だが、次の tier は前の tier が crates.io index に
反映されてから実行する** (数秒〜数分)。

```
依存グラフ:

  utsuroi ─────────────────┐
  arika ──────┬─────────────┤
  rrd-wasm    │             │
  orts-plugin-sdk           │
              │             │
              ▼             │
            tobari          │
              │             │
              ▼             ▼
             orts (← arika, utsuroi, tobari)
              │
              ▼
           orts-cli (← orts + 全 crate)
```

```sh
# Tier 1: Leaf crates (workspace dep なし、並列 OK)
cargo publish -p utsuroi
cargo publish -p arika
cargo publish -p rrd-wasm
(cd plugin-sdk && cargo publish)
# → crates.io index 反映を待つ

# Tier 2: tobari (← arika)
cargo publish -p tobari
# → crates.io index 反映を待つ

# Tier 3: orts (← arika, utsuroi, tobari)
cargo publish -p orts
# → crates.io index 反映を待つ

# Tier 4: orts-cli (← orts + 全 crate、viewer embed あり)
# 前提: pnpm --filter uneri build && pnpm --filter orts-viewer build 済み
cargo publish -p orts-cli
```

**注意**: publish 済み crate は取り消せない。途中で失敗した場合、
修正版を bump して再 publish する。

**Rate limit**: crates.io は新規 crate publish にレート制限がある
(10 分間に 5 crate 程度)。Tier 1 で 4 crate publish すると Tier 2-3
で 429 が返る場合がある。エラーメッセージの `try again after` の時刻
まで待ってから retry する。

### Pre-publish dry-run

real publish 前に dry-run で metadata validity を確認:

```sh
# Leaf crates (成功するはず)
cargo publish --dry-run --allow-dirty -p utsuroi
cargo publish --dry-run --allow-dirty -p arika
cargo publish --dry-run --allow-dirty -p rrd-wasm
(cd plugin-sdk && cargo publish --dry-run --allow-dirty)

# Mid-tier / CLI は workspace dep が crates.io に存在しないと fail するので、
# 一連の real publish が終わってから確認することになる。
# 代わりに cargo package --no-verify で tarball 内容のみ確認可能。
```

## npm publish

```sh
# uneri
(cd uneri && pnpm publish --access public)

# starlight-rustdoc (publishConfig で main を ./dist/index.js に切り替え)
pnpm --filter starlight-rustdoc build   # dist を最新に
(cd starlight-rustdoc && pnpm publish --access public)
```

### 注意: npm registry 設定

local の `~/.npmrc` / `npm config get registry` が `https://registry.npmjs.org/`
を向いていることを確認。sksat の work registry (`npm.flatt.tech` 等) が設定
されていると `pnpm publish --dry-run` の出力で "Publishing to npm.flatt.tech"
と出る — その場合 `--registry https://registry.npmjs.org/` を明示指定する。

## Binstall での install 手順 (エンドユーザー向け)

```sh
# Pre-publish (private repo)
cargo binstall --git https://github.com/sksat/orts orts-cli --version 0.1.0

# crates.io publish 後 (GA 以降)
cargo binstall orts-cli
```

`cli/Cargo.toml` の `[package.metadata.binstall]` で pkg-url template が
定義されているので、どちらの経路でも同じ GH Release tarball を取得する。
