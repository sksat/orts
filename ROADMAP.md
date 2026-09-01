# orts Roadmap

未実装の拡張計画。
完了した項目はこのファイルから削除する。

## 複数衛星

- **Scheduler の CLI / config 統合**: 複数衛星の積分レジーム (独立 / 同期 / 結合) とヒステリシス付き遷移は orts 内部に実装済みだが、CLI からは使えず常に独立レジーム (IndependentGroup) で動く。config でレジームと遷移閾値を指定できるようにする
- **レジーム遷移の実ミッション検証**: 衛星分離のような不連続イベント (インパルス、質量ジャンプ、積分器リスタート) を含むシナリオで、レジーム遷移が安定して動くことを確かめる
- **1000+ 衛星への対応**: 衛星ごとの WASM instance 生成コストを下げる wasmtime の pooling allocator を導入する。効果をまず benchmark で確かめてから着手する

## プラグイン

- **controller trait の統一**: plugin 用の PluginController と既存の DiscreteController が意図的に並存している。native 実装の移行が済んだら 1 つの trait に統一する
- **暴走 guest の決定論的な打ち切り**: 無限ループする guest を打ち切る手段が今は無い。wasmtime の fuel budget (実行命令数の上限) なら、壁時計ベースの打ち切りと違って何回実行しても同じ位置で止まり、決定論を保てる
- **事前コンパイル済みプラグイン (`.cwasm`) の配布**: 現在の配布は portable な `.wasm` のみで、ロード時にコンパイルが走る。事前コンパイルすれば起動が速くなり、実行側から compile 層 (Cranelift) を抜いて配布サイズも減らせる (`plugin-wasm-runtime-only` feature)。ただし `.cwasm` のロードは任意コード実行と等価なので、CI で生成・検証する trusted な配布経路の設計が前提
- **第 2 backend**: WASM 以外の backend (pure Rust の embedded script 系、Rhai 等) を評価して 1 つ選ぶ。既存 Rust 実装との同等性テスト (oracle) を pass したら完了
- **WASM guest のホットリロード**: guest を差し替えるには今は simulation の再起動が必要。`snapshot_state` / `restore_state` で内部状態を引き継ぎ、走らせたまま差し替えられるようにする
- **thruster コマンドの拡張**: guest が指令できるのは今は throttle だけ。impulsive delta-v (瞬時の速度変化) や force 直接指定の variant を足す
- **stream-io の replay**: 実機ツールと繋いだセッションで流れた byte 列を tick 付きで記録し、後から決定論的に再生できるようにする
- **msg-io の対話入力**: コマンド入力は今は config に書いた時刻シーケンスのみ。viewer から WebSocket で対話的に送れるようにする。到着タイミングが非決定論的なので、tick 境界で配送を確定してコマンドタイムラインとして記録し (そのままリプレイに使える)、認証も含めて設計する
- **RF sideband**: stream-io は byte 列より下 (変調、受信品質) を扱わない。link 品質 (lock, SNR, Doppler 等) を模擬したくなったら、modem を 1 デバイスとしてモデル化する別チャネルを設ける。必要な observable が固まってから設計する

## 惑星間遷移

中心天体の切り替え (地球 → 月、地球 → 太陽など) を段階的に導入する。現状は中心天体固定で、SOI (影響圏) 逸脱の検知も無い。

- **手動切替**: SOI 脱出をイベント検出して積分を停止し、ユーザーが座標変換して新しい系で再開する
- **自動監視**: 摂動強度比から中心天体の妥当性を常時監視し、SOI 境界接近で警告、オプションで自動切替
- **完全 N 体**: 太陽系規模。中心天体を固定せず、慣性系で全天体の重力を直接計算する

切り替え実装時の設計制約は [DESIGN.md](DESIGN.md) の「ミッション規模と力学モデル」を参照。

## Viewer

- **複数 source の同時接続・比較表示**: live と replay を並べて見るような使い方。全 source が同一パイプラインに流れる現在の設計は、この拡張を見込んだもの
