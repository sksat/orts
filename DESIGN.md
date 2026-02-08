# Orts Design Doc

Orts は主に軌道計算を目的とした数値計算・数値最適化プラットフォーム。

開発は TDD で実施する。
TDD を効率化するために、以下のアプローチを取る。
- 数値積分や座標変換といったロジックの unit test を積極的に行い、モジュールの挙動を検証してから統合する
- GMAT や Orekit などの実績のあるシミュレータを参照実装として用い、E2E でのブラックボックステストを行う
- SSO 軌道や衛星コンステレーション、太陽系のN年間の軌跡、ラグランジュ点、スイングバイなどといった典型的な問題をテストケースとして用意する
- 常にフルスペックの計算をするのは過剰なので、系や計算ロジックや精度は切り替え可能な設計とする
  - 例えば SSO の計算をする時はせいぜい地球 - 月 - 太陽の系があればよい
  - 太陽輻射圧や大気抵抗を詳細に計算するか、抵抗系数のみのような簡易的なものにするか

実装は主に Rust で行う。
座標変換や数値積分など、ある程度の単位でライブラリを分割して開発し、それぞれごとに unit test も行う。

シミュレータ本体だけでなく、Web ベースのシミュレータの real-time viewer も開発する。
viewer は React + TypeScript で実装し、Vite による hot reload 開発に対応する。

viewer は以下の2つのモードに対応する:
- **リプレイモード**: 記録済みの CSV データを読み込み、時間制御（再生/一時停止/速度調整/シーク）付きで再生する
- **リアルタイムモード**: Rust シミュレータを WebSocket サーバーとして実行し、計算結果をリアルタイムにストリーミング表示する

リアルタイムモードでは、Rust 側に WebSocket サーバー機能を追加し、シミュレーション結果を逐次 viewer に送信する。

ラフなところから精度を上げていくためにも、はじめはシンプルな2体問題や3体問題を低精度で実装する。
viewer についてもシンプルなものをまず実装する。
次にテストのためのインフラを作り、E2E でのテストや精度検証を可能にする。
Playwright を用いて viewer の E2E テスト環境も用意する。

テストのためのインフラを用意したら TDD を本格的に進め、それによって精度の向上や対応する問題を増やしていく。
開発にあたっては、責務の分離を徹底することで並列での開発を可能にすること。

また、シミュレータは CLI で実行可能にしておく事でシンプルな E2E テストを可能にする。

## データモデル

Rerun (https://rerun.io/) のデータフォーマット設計を参考に、ECS (Entity-Component-System) ベースのデータモデルを採用する。

### Entity-Component-Archetype

- **Entity**: 階層パスで識別されるオブジェクト（例: `/world/earth`, `/world/sat/iss`）
- **Component**: データの最小単位。`Position3D`, `Velocity3D`, `GravitationalParameter` など
- **Archetype**: Component のバンドル。`OrbitalState`（position + velocity）、`CelestialBody`（mu + radius）など

### タイムライン

1つのデータに複数のタイムラインを紐付け可能:
- シミュレーション時刻（秒）
- ステップ番号（シーケンス）
- 壁時計（オプション）
- カスタムタイムライン

Static Data（天体パラメータなど）はタイムラインを持たず、全時刻で有効。

### Recording

シミュレーション結果は `Recording` に蓄積される。Recording は Entity ごとに static データと temporal データ（列指向 SoA レイアウト）を保持する。

### ファイルフォーマット

デフォルトの保存形式は Rerun の `.rrd` フォーマット（Apache Arrow IPC ベース、MIT/Apache 2.0 デュアルライセンス）。
Rerun SDK を logging-only モードで使用し、Rerun Viewer からの再解析やクエリも可能。

## CLI

CLI はサブコマンド構造を持つ:

```
orts-cli run [OPTIONS]                           # シミュレーション実行 → .rrd 保存（デフォルト）
orts-cli run --output stdout --format csv        # CSV を stdout に出力
orts-cli serve [OPTIONS]                         # WebSocket サーバー
orts-cli convert <input> --format csv            # フォーマット変換（rrd → csv）
```

### run オプション
- `--altitude <km>` (default: 400) — 軌道高度
- `--body <name>` (default: earth) — 中心天体
- `--dt <seconds>` (default: 10) — 積分時間刻み
- `--output-interval <seconds>` — 出力間隔（dt と独立）
- `--output <path|stdout>` (default: output.rrd) — 出力先
- `--format <rrd|csv>` (default: rrd) — 出力フォーマット

### serve
WebSocket サーバーモード。将来的には Web クライアントからシミュレーション条件を指定して実行する機能を持つ。

### convert
フォーマット変換。rrd → csv などをサポート。