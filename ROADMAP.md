# orts Roadmap

未実装の拡張計画。
各項目は目的と完了条件を書き、具体的なタスク分解や進捗は GitHub issue に置く。
完了した項目はこのファイルから削除する (設計上の決定は [DESIGN.md](DESIGN.md) に、経緯は git 履歴に残る)。

## 複数衛星

- **Scheduler の CLI / config 統合**: 3 レジーム (独立 / 同期 / 結合) とヒステリシス付き遷移は実装済みだが、CLI は現在 IndependentGroup を直接使っている。config からレジームと閾値を指定して Scheduler を使えるようにする
- **レジーム遷移の実ミッション検証**: 分離イベント (インパルス、質量ジャンプ、積分器リスタート) を含むシナリオでの検証
- **1000+ 衛星のスケール**: pooling allocator の導入。まず benchmark で必要性を確認してから着手する

## プラグイン

- **PluginController と DiscreteController の統一**: 現在は意図的に並存している (plugin 側は TickInput / Command を固定)。native 側の利用状況を見て 1 trait に統一する
- **consume_fuel による interruption の決定論化**: 現在の決定論は Pulley interpreter と順次呼び出しで担保している。fuel budget を導入すれば、暴走 guest の interruption も決定論化できる (壁時計ベースの epoch_interruption は非決定論的なので使わない)
- **`.cwasm` 配布**: `Engine::precompile_component` による事前コンパイルと deserialize path。untrusted な `.cwasm` のロードは任意コード実行リスクがあるため、trusted artifact 境界 (CI で生成・検証) の設計が前提
- **plugin-wasm-runtime-only feature**: Cranelift を抜いた最小 runtime で配布サイズを削減する (`.cwasm` 配布とセット)
- **第 2 backend**: pure Rust の embedded script 系 (Rhai 等) を評価して選定する。WASM backend との oracle 同等性テストを pass することが完了条件
- **WASM guest のホットリロード**: 現在は restart 運用。`snapshot_state` / `restore_state` で同一 guest バイナリ間の状態引き継ぎを行う
- **thruster コマンドの拡張**: 現在は throttle のみ。impulsive delta-v や force ベースの variant を追加する
- **stream-io replay モード**: 録った byte chunks を tick-stamp して決定論再生する
- **msg-io の WebSocket 対話入力**: viewer からの運用コンソール。tick 境界での gate、コマンドタイムラインの記録 (リプレイ用)、認証を含めて設計する
- **RF sideband**: byte stream の継ぎ目で失われる RF observable (soft-decision, lock, SNR, Doppler) を modem のデバイス模型として扱う。必要な observable が固まってから設計する

## 惑星間遷移と SOI

中心天体の切り替えを段階的に導入する。

- **Phase 2 (手動切替)**: イベント検出で SOI 脱出を検知して積分を停止し、ユーザーが座標変換 + 新しい OrbitalSystem で再開する
- **Phase 3 (自動監視)**: 摂動強度比で中心天体の妥当性を継続監視し、SOI 境界接近で警告、オプションで自動切替
- **Phase 4 (完全 N 体)**: 太陽系規模。慣性系で全天体の重力を直接計算する

切り替え実装時の設計制約は [DESIGN.md](DESIGN.md) の「ミッション規模と力学モデル」を参照。

## Viewer

- **複数 source の同時接続・比較表示**: 全 source が同一パイプラインに流れる現在の設計は、この拡張を見込んだもの
