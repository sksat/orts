# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

orts is a numerical computation and optimization platform primarily for orbital mechanics.

- 設計意図: [DESIGN.md](DESIGN.md)(日本語)/ 全体構造: [ARCHITECTURE.md](ARCHITECTURE.md)
- crate / package の一覧と役割: [README.md](README.md) の Project Structure の表を参照

## Languages

- **Rust**: コアシミュレーションプラットフォーム(Cargo workspace)
- **TypeScript/React**: リアルタイムビューアなど(pnpm workspace)
- **Python**: examples/ や tools/ の補助スクリプト。環境は uv で管理する

## Build & Test

- `plugin-sdk/examples/` は独立した workspace(`cargo component build`, target は `wasm32-wasip1`)。`cargo test --workspace` には含まれない
- 一部の crate は no_std / wasm 向けの check が CI にある(no_std の feature 構成ごとの clippy は lint job、wasm32 は viewer-build など wasm-pack を使う job)。何をどう check するかは `.github/workflows/ci.yml` が正。該当 crate を変更したら同じ check をローカルでも回す

## Footguns

- `cargo test -p orts-cli` は `viewer/src/protocol/generated/` の TypeScript bindings を再生成する(`TS_RS_EXPORT_DIR` @ `.cargo/config.toml`)。CI が diff を enforce するので、protocol 型を変更したら再生成して commit する
- `.cargo/config.toml` が mold + clang linker を設定している。グローバルな `RUSTFLAGS` が非空だとこの設定は黙って無効化される
- リリース手順は [RELEASING.md](RELEASING.md) を参照

## Development Workflow

- アーキテクチャレベルの変更は DESIGN.md を先に更新してから実装する
- 設計判断を伴う実装(新しい module 構成、trait / public API の設計、設計の選択肢が複数あるとき)は、着手前に smart-friend で独立レビューにかけ、レビューが通るまで設計を見直す
- TDD-first: 統合の前にユニットテストで挙動を検証する。GMAT / Orekit を参照実装とした E2E 検証(fixture 生成は tools/)
- commit 前に `cargo fmt` / `cargo clippy --workspace -- -D warnings` / 関連テスト / `pnpm lint` を通す
- ロジック・API・設計に触れる変更は commit 前に code-review skill で外部レビューを受ける(typo 修正や機械的な置換だけの commit は省略してよい)。指摘対応後は re-review し、通ってから commit する
- push 後は CI 結果の確認までを一連の作業とする
- WebSocket 通信・データフロー・UI 統合など mock しにくい部分を変更したら Playwright E2E も実行する(Playwright は CLI を使う。MCP ツールではない)

## Testing Rules

- テストは「何を検証したいか」を明確にする。トートロジーになっているテストは書かない
- 不具合を見つけたら、まず再現テストを書いてから修正する(regression 防止)
- テスト失敗やバグを「既存の問題」「flaky」と判断する場合は、再現確認などの根拠を示す
- テストやテストモジュールの削除は、事前に対象・理由・カバー状況を列挙して user のレビューを受ける。依存関係の問題は削除ではなく依存の追加で解決する
- 挙動を保存するリファクタでは characterization test で既存挙動を固定する。境界値と非有限値(`NaN`, `±∞`)も含める

## Working Rules

- 重いコマンド(`cargo test --workspace` など)はフルログをファイルに保存してから必要箇所を抽出する。最初から `| tail` で切り詰めない
- サイズの大きいバイナリ生成物(gif、画像など)は Claude 自身で Read せず、人間の目視確認に委ねる。大きいバイナリファイルは commit しない
- 命名は実態・意味に忠実にする(真値かノイズ入りか、暗黙のデフォルトの有無などが名前から読み取れるように)
- 対応を先送りする判断をしたら TODO コメントを残す
- マジックナンバーは定数として定義するか、根拠をコメントで書く

## Documentation

- 技術的な主張(CHANGELOG、docs など)は実装・テストと照合して裏取りしてから書く
- 日本語文書でも技術用語は英語表記のまま使う(crate, workspace, commit)。カタカナ化しない

## Dependencies

- 新しいライブラリを追加する際は、最新の安定バージョンを調べてから指定する。古いバージョンを指定しない
