---
name: smart-friend
description: |
  A knowledgeable neighbor who offers a fresh perspective on architecture, design decisions, and test strategy.
  Use proactively when a second opinion would improve quality, or when the user says "別の視点が欲しい",
  "設計を相談したい", "相談", "smart-friend に聞いて", or asks for feedback on their approach.
---

# smart-friend: External AI Design Reviewer

Codex CLI (`codex exec`) を使って、設計やテスト戦略について外部 AI から独立したレビューを得る。
Claude 単独では得られない「別の視点」を提供することが目的。

## 使い方

```bash
codex exec --sandbox read-only -m gpt-5.4 "<prompt>"
```

プロンプトが長い場合や `$` やバッククォートを含む場合は、heredoc で stdin から渡す:
```bash
cat <<'EOF' | codex exec --sandbox read-only -m gpt-5.4 -
<prompt>
EOF
```

## プロンプトの組み立て方

レビュー対象と観点を明確にしたプロンプトを組み立てる。
Codex はワーキングディレクトリのファイルを読めるので、関連ファイルを読むよう指示する。

構成:
1. **コンテキスト**: 関連ファイルを読む指示（DESIGN.md、ソースコードなど）
2. **レビュー対象**: ユーザーの提案や設計案を具体的に記述
3. **レビュー観点**: 何を見てほしいか

例:
```
以下の設計提案をレビューしてください。

## コンテキスト
DESIGN.md を読んで、プロジェクトの全体設計を把握してください。

## レビュー対象
BusAccessor trait の API 設計として以下を検討しています:
<ユーザーの提案>

## レビュー観点
- DESIGN.md の方針との整合性
- 見落としているエッジケースや懸念点
- 代替案があればその trade-off
```

## レビュー結果の扱い方

Codex のレビュー結果を受けて、状況に応じた対応を取る:

- **指摘がもっともで即座に対応できる場合**: レビュー内容を要約してユーザーに伝えつつ、修正を進める
- **議論が必要な指摘の場合**: Codex の意見と自分（Claude）の見解を両方提示し、ユーザーと議論する
- **意見が分かれる場合**: 両方の視点と trade-off を整理して、ユーザーの判断を仰ぐ

重要なのは、Codex の回答を鵜呑みにせず、自分の視点も持った上で建設的に扱うこと。

## 注意事項

- 必ず `--sandbox read-only` を付けてファイル変更を防ぐ
- codex の実行には時間がかかることがある。Bash の timeout は 300000 (5分) を設定する
- ユーザーが別のモデルを指定した場合はそれに従う
