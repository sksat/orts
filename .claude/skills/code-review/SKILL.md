---
name: code-review
description: |
  Run an external AI code review to get independent feedback on code changes.
  Use this skill when the user asks for a code review, says "コードレビューして", "レビュー", "変更を見て",
  "code review", or wants external feedback on their recent commits or uncommitted changes.
---

# code-review: External AI Code Reviewer

Codex CLI (`codex review`) を使って、コード変更に対する独立したレビューを得る。
diff の取得は codex が自動で行う。

## 使い方

```bash
# 未コミットの変更をレビュー
codex review -c model=gpt-5.4 --uncommitted

# 特定ブランチとの差分をレビュー
codex review -c model=gpt-5.4 --base main

# 特定コミットをレビュー
codex review -c model=gpt-5.4 --commit <SHA>
```

カスタムプロンプトで追加のレビュー指示を渡すこともできる（対象指定なしの自由プロンプトモード）:
```bash
codex review -c model=gpt-5.4 "TDD-first の方針に沿っているかも確認してください"
```

**制約: `[PROMPT]` と対象指定オプション (`--uncommitted`/`--base`/`--commit`) は排他。同時に使うとエラーになる。**

## 対象の選び方

- ユーザーが対象を指定しない場合:
  - 未コミットの変更があれば `--uncommitted`
  - クリーンなワークツリーの場合は `--base main` でブランチ全体をレビュー（`--commit HEAD` だと直近1コミットしか見ない）
- ブランチの差分全体をレビューしたい場合: `--base main`
- 特定コミットだけをレビューしたい場合: `--commit <SHA>`
- `--title` はレビューサマリーの表示ラベル。レビュー指示ではない

## レビュー結果の扱い方

- 指摘がもっともで即座に対応できる場合: 要約してユーザーに伝えつつ修正を進める
- 議論が必要な指摘の場合: Codex の意見と自分（Claude）の見解を両方提示し、ユーザーと議論する
- 意見が分かれる場合: 両方の視点と trade-off を整理して、ユーザーの判断を仰ぐ

Codex の回答を鵜呑みにせず、自分の視点も持った上で建設的に扱うこと。

## 注意事項

- モデル指定は `-m` ではなく `-c model=gpt-5.4` を使う（`codex review` の制約）
- codex の実行には時間がかかることがある。Bash の timeout は 300000 (5分) を設定する
- ユーザーが別のモデルを指定した場合はそれに従う
