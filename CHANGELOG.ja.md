# Changelog (日本語)

[Keep a Changelog](https://keepachangelog.com/ja/1.1.0/) に緩く準拠。
[Semantic Versioning](https://semver.org/) で versioning。

orts は マルチパッケージ workspace (crates.io Rust crate + npm package)。
全パッケージを同一バージョンでリリースし、セクションはパッケージ別に分割。

## [Unreleased]

### `orts` (Rust, crates.io)

#### Added
- 円錐半影 (conical penumbra) モデルによる日食・影ジオメトリ計算
- Per-device アクチュエータコマンド
  - MTQ・RW を個別デバイスリストとして管理し、デバイス単位で指令を送信
- マルチインスタンスセンサ: sensor を `Vec` ベースに変更し任意の個数に対応
- RW モーター一次遅れ (first-order lag) モデル
- RW 速度指令 / トルク指令バリアントと `MtqCommand` variant
- 非直交 RW/MTQ レイアウト向けの擬似逆行列トルク・ダイポール配分
- Fine/Coarse バリアント付きサンセンサモデル
- Controlled simulation の姿勢・コマンド・テレメトリログ
  - 動的 CSV カラム生成

#### Changed
- アクチュエータ telemetry をアクチュエータ種別横断で統一的に構造化
- `orts convert` を姿勢・コマンド・テレメトリを含むフルデータ出力に拡張
- CSV metadata・satellite 出力を `SimMetadata::write_csv_header` /
  `write_satellite_csv` に統一

### `orts-plugin-sdk` (Rust, crates.io)

#### Added
- `no_std` サポート
  - 標準ライブラリなし (allocator 不要) でコンパイル可能
  - オプションの `alloc` feature flag で `no_std` 下でのヒープ使用に対応
- 新規 example: `nos3-adcs` — NOS3 `generic_adcs` WASM plugin (SILS デモ)
  - 全モードテスト、IGRF 統合、可視化スクリプト、CI workflow

#### Changed
- example plugin を `plugin-sdk/examples/` workspace に移動

### `arika` (Rust, crates.io)

#### Added
- `no_std` + `alloc` サポート (tiered feature hierarchy)
  - no alloc: core math (座標フレーム、エポック演算、解析 ephemeris、
    測地変換、IAU 2006 歳差・章動)
  - `+ alloc`: Horizons、EopTable、HorizonsMoonEphemeris
  - `+ std`: `Epoch::now()`、file I/O、fetch-horizons
  - `libm` ベースの `F64Ext` trait で no_std 環境での超越関数を提供

#### Changed
- ブラウザ向け WASM facade を `arika-wasm` crate に分離

### `utsuroi` (Rust, crates.io)

#### Added
- `no_std` サポート — pure math でヒープ allocation 不要のため
  `alloc` feature は不要。`libm` ベースの `F64Ext` trait を追加

### `tobari` (Rust, crates.io)

#### Added
- `no_std` + `alloc` サポート (tiered feature hierarchy)
  - no alloc: Exponential、Harris-Priester、TiltedDipole、
    SpaceWeather traits、ConstantWeather
  - `+ alloc`: NRLMSISE-00、IGRF、CSSI/GFZ parsing
  - `+ std`: file I/O、fetch、OnceLock

#### Changed
- ブラウザ向け WASM facade を `tobari-wasm` crate に分離
- `Nrlmsise00` を `SpaceWeatherProvider` 上で generic 化 (alloc-free)
- IGRF / NRLMSISE-00 の内部ストレージを `Vec` → 固定サイズ配列に変更
  (alloc-free)

### `starlight-rustdoc` (npm)

#### Added
- 生成された API ドキュメントページに feature-gate バッジを表示

## [0.1.1](https://github.com/sksat/orts/releases/tag/v0.1.1)

### `orts-cli` (Rust, crates.io, binary)

- `cargo install` 時の `include_bytes!` texture パスを修正。build.rs が
  `viewer/public/textures/` → `cli/textures/` にコピーし、
  `CARGO_MANIFEST_DIR` ベースで参照する形に変更 (`viewer-dist/` と同じ pattern)。

### `uneri` (npm: `@sksat/uneri`)

- npm package 名を `uneri` → `@sksat/uneri` (scoped package) に変更。
  npm が既存パッケージとの類似名で unscoped 名を拒否したため。

## [0.1.0](https://github.com/sksat/orts/releases/tag/v0.1.0)

### `orts` (Rust, crates.io)

- 軌道力学シミュレーションの core library: `OrbitalState` (位置+速度),
  `AttitudeState` (quaternion + 角速度), `SpacecraftState` (両方の結合)。
  `HasOrbit`, `HasAttitude`, `HasMass` trait bounds による capability
  ベースの model 合成。
- 軌道力学: 二体問題, Brouwer 平均軌道要素伝播, 重力球面調和関数
  (最大 16 次), TLE/SGP4 相当パス。
- 摂動力 model: 大気抵抗 (`tobari` 経由の plugin 対応の密度),
  日食影付き太陽輻射圧, 第三体重力 (太陽/月), スケジュール/定常推力。
- 姿勢力学と制御: 剛体 dynamics, 重力傾斜・空力トルク,
  reaction wheel, thruster, 表面パネル, B-dot detumbler ・ PD
  tracker ・ nadir/慣性指向を含む controller。
- sensor model: 磁気センサ, gyroscope, star tracker
  (オプションのノイズ注入付き)。
- wasmtime による WebAssembly Component Model plugin runtime
  (`plugin-wasm` feature)。実行時に guest controller を load 可能。
  オプションの fiber ベース非同期 backend (`plugin-wasm-async`) で
  単一 worker スレッド上で多数の衛星を多重化。
- Rerun RRD への記録・telemetry。複数 frame での位置/速度/姿勢/角速度
  の構造化 archetype。
- 宇宙機制約に基づくイベント検出と積分終了条件 (デオービット,
  遠地点/近地点通過, 地上コンタクト)。
- オプション feature: `fetch-weather` (CSSI/GFZ 宇宙天気 download,
  `tobari/fetch` 経由), `fetch-horizons` (JPL Horizons ephemeris HTTP
  取得, `arika/fetch-horizons` 経由)。
- workspace crate `arika` (frame/エポック/ephemeris),
  `utsuroi` (積分器), `tobari` (大気+磁場) に依存。
- `orts/examples/` にシミュレーション例を同梱:
  - `apollo11` — Apollo 11 全行程の軌道伝播と 3D 可視化。JPL Horizons
    参照軌道で検証。
  - `artemis1` — NASA Artemis 1 coast feasibility spike (2022-11-16 →
    2022-12-11 ミッションの主要 3 フェーズ)。Earth-centric DOP853 で
    伝播し Horizons Orion target `-1023` と比較。
  - `orbital_lifetime` — 大気抵抗+平均軌道要素伝播による長期減衰
    シミュレーション。
  - `wasm_bdot_simulate` / `wasm_pd_rw_simulate` — `orts-example-plugin-*`
    WASM guest を load して detumbling / RW 制御シナリオを E2E 実行する
    host 側サンプル。

### `orts-cli` (Rust, crates.io, binary)

- 4 つの subcommand を持つ `orts` バイナリ:
  - `orts run` — batch simulation、`.rrd` (デフォルト) または
    `.csv` を出力。
  - `orts serve` — ポート 9001 で WebSocket telemetry サーバ +
    組み込み 3D ビューア SPA (`http://localhost:9001`)。
  - `orts replay` — 記録済み `.rrd` を組み込みビューアで streaming。
  - `orts convert` — `.rrd` ↔ `.csv` format 変換。
- CLI フラグ: 高度, 中心天体 (Earth/Moon/Mars), 時間刻み, 出力間隔,
  エポック (ISO 8601), TLE 入力 (ファイルまたは `--tle-line1`/`--tle-line2`),
  YAML config, WASM plugin controller 指定。
- 組み込み 3D ビューア (`viewer` feature, デフォルト ON): React +
  Three.js + `@react-three/fiber` SPA を `rust-embed` でバイナリに同梱。
  同一 WebSocket プロセスから配信し、setup 不要で可視化。
- マルチ衛星 plugin backend: 衛星ごとスレッド (`sync`) または
  fiber 多重化 (`async`) runtime。constellation 規模のシナリオに
  対応。
- `[package.metadata.binstall]` 設定済み。
  `cargo binstall orts-cli` でプリビルド済み GitHub Release tarball を
  直接取得可能 (コンパイル不要)。`x86_64-unknown-linux-gnu` と
  `x86_64-unknown-linux-musl` (完全静的リンク) の両ターゲット。
- single binary 配布: simulator, WebSocket サーバ, ビューア SPA を
  まとめて同梱。

### `orts-plugin-sdk` (Rust, crates.io)

- Component Model 向け orts WASM plugin guest 開発 SDK。
  `cargo component` でビルド。
- callback 型 `Plugin<I, C>` trait: `sample_period()`, `init(config)`,
  `update(input) -> Option<Command>`, オプションの `current_mode()` を
  実装。`orts_plugin!(MyController)` macro で world 準拠の `Guest` impl
  に変換 (tick loop, モード報告, エラー伝播)。
- main-loop 型: カスタム `impl Guest` から `wait_tick()` /
  `send_command()` を呼ぶ逐次的な "phase 1 → wait → phase 2" controller。
- `I`/`C` は generic で、デフォルトは WIT 生成の `TickInput`
  (軌道/姿勢状態+センサ読み取り) と `Command` (thruster 推力,
  磁気トルカ dipole, reaction wheel トルク)。
- runtime 依存なし — macro は consumer の `bindings` module
  (`cargo component` が orts plugin WIT world から生成) を参照。
- `plugins/` にサンプル plugin guest crate を同梱 (独立 cargo
  workspace, crates.io 非公開, ユーザーが自作 controller を書く際の
  reference 実装):
  - `orts-example-plugin-bdot-finite-diff` — main-loop 型 B-dot
    detumbling controller。磁気センサの有限差分 `dB/dt` 推定を使用。
  - `orts-example-plugin-pd-rw-control` — callback 型 PD 姿勢
    tracker。left-invariant quaternion 誤差で reaction wheel 駆動。
  - `orts-example-plugin-pd-rw-unloading` — callback 型 PD 姿勢
    制御 + 磁気トルカによる reaction wheel 運動量アンローディング。
  - `orts-example-plugin-detumble-nadir` — callback 型 detumble →
    nadir 指向モード遷移。ユーザー定義の収束条件付き。

### `arika` (Rust, crates.io)

- phantom 型 frame system: frame-tagged 3D vector `Vec3<F>` と
  frame transform `Rotation<From, To>`。frame marker: `SimpleEci`,
  `SimpleEcef` (ERA のみの回転), `Gcrs`, `Cirs`, `Tirs`, `Itrs`
  (IAU 2006 CIO チェーン), `Rsw` (局所軌道 radial/along-track/cross-track),
  `Body` (機体固定)。
- IAU 2006 / 2000A_R06 CIO ベースの地球回転: 歳差, 章動,
  CIP X/Y/s 系列評価器, EOP provider trait による完全な
  `Rotation<Gcrs, Itrs>` 合成。
- scale-tagged `Epoch<S>` (`S ∈ {Utc, Tai, Tt, Ut1, Tdb}`) —
  コンパイル時に時刻 scale の暗黙的混合を防止。scale 間変換は
  明示的 method (`to_tai()`, `to_tt()` 等)。
- `EphemerisProvider` trait による天体 ephemeris: 太陽/月/惑星の
  低精度 Meeus 解析 model、およびオプションの JPL Horizons vector
  テーブル parser (Hermite 補間 + disk cache, `fetch-horizons`
  feature)。
- WGS84 測地 ↔ ECEF 変換, RSW 軌道 frame 計算
  (`rsw_quaternion(pos, vel)`), body-to-RSW 姿勢変換。
- `wasm` feature: `wasm-bindgen` 経由で `wasm32-unknown-unknown` に
  コンパイル。ブラウザビューアがネイティブ往復なしで ECI ↔ ECEF 変換と
  ephemeris 検索を実行可能。

### `utsuroi` (Rust, crates.io)

- 統一的 `Integrator` trait: multi-step 積分, イベント検出,
  NaN/Inf guard (`integrate_with_events()`)。
- 固定ステップ積分器: RK4 (4 次 Runge-Kutta), Störmer-Verlet
  (2 次 symplectic, 長期エネルギー保存), Yoshida 4/6/8 次
  symplectic 合成。
- 適応 step size 積分器: Dormand-Prince RK5(4)7M (FSAL, DP45) と
  DOP853 (Hairer/Nørsett/Wanner 8 次 RK8(5,3))。
- trait ベースの問題定義: `DynamicalSystem` が微分を定義、`OdeState` が
  BLAS ライクな演算 (`axpy`, `scale`, `error_norm`) を提供。solver code は
  任意の状態次元に対して generic。
- Pure Rust, LAPACK/BLAS 依存なし。

### `tobari` (Rust, crates.io)

- `AtmosphereModel` trait 背後の大気密度 model:
  `Exponential` (US Standard Atmosphere 1976, 高度のみ),
  `HarrisPriester` (太陽位置による日変化),
  `Nrlmsise00` (太陽/地磁気活動入力付き完全 NRLMSISE-00 経験 model)。
- IGRF-14 球面調和展開による地磁気場 (`Igrf`, 1-13 次設定可能)。
  同梱の 2020 DGRF + 2025 IGRF + 永年変化係数。実行時にカスタム係数
  注入可能。傾斜 dipole 近似も利用可能。
- `SpaceWeatherProvider` trait と組み込み provider: `ConstantWeather`
  (固定 F10.7/Ap), `CssiSpaceWeather` (CelesTrak CSSI CSV parser),
  `GfzSpaceWeather` (GFZ Kp/Ap/F10.7 parser)。
- デフォルトの `fetch-igrf` feature は同梱係数でビルド。オプションの
  `fetch` feature で CSSI/GFZ データを HTTP 経由でライブ取得。
- `wasm` feature: `wasm-bindgen` 経由で密度・磁場検索を公開。
  ブラウザ側の大気/磁場 visualizer 向け。
- frame-tagged 位置と測地変換のために `arika` に依存。
- 同梱デモ: `tobari-example-web` (`tobari/examples/web/` 配下の private
  npm workspace) — React + Three.js ブラウザデモ。`tobari` + `arika`
  WASM ビルドで大気密度, IGRF 地磁気場, 宇宙天気データを完全に
  ブラウザ内で可視化。npm 非公開; 統合 smoke test および docs サイトの
  組み込みライブデモとして使用。

### `rrd-wasm` (Rust, crates.io)

- WebAssembly 対応の Rerun RRD decoder。Rerun SDK の decoder 部分
  (`re_log_encoding`, `re_chunk`, `re_log_types`, `re_sdk_types`) をラップ。
- `wasm` feature: `parse_rrd(bytes)` entry point を公開。
  `serde-wasm-bindgen` 経由で serializable な構造化
  `{metadata, rows}` object を返す。ブラウザビューアが Web Worker
  上で `.rrd` byte stream を decode 可能 (ネイティブ Rerun Viewer 不要)。
- metadata: エポック (ユリウス日), 重力 parameter μ, 天体半径,
  天体名, 軌道高度, 周期。
- 行 payload: timestamp, 位置/速度 (km, km/s), entity パス,
  オプションの quaternion / 角速度。
- orts 固有のシミュレーションロジックへの依存なし — 純粋なデータ
  serialization 層。

### `uneri` (npm)

- [uPlot](https://github.com/leeoniya/uPlot) をラップした React
  `<TimeSeriesChart />` component。リアルタイム時系列可視化、
  legend での series 分離。
- schema-driven API: column (`DOUBLE`, `INTEGER`, `FLOAT`, `BIGINT`) と
  派生 SQL 式を宣言。uneri がテーブル作成, ingest, ブラウザ内での
  query 時 downsampling を処理。
- `IngestBuffer<T>` staging buffer。drain pattern で
  stream 到着 (WebSocket, ファイル等) と DuckDB INSERT 間隔を分離。
- `useTimeSeriesStore` hook: リアルタイム tick loop (蓄積 → INSERT →
  設定可能な refresh rate での定期的 downsample query)。
- query 時の時間 bucket downsampling。データ密度に関係なく
  チャートカバレッジを比例的に維持 (疎/密混在でも視覚的にバランス)。
- `ChartDataWorkerClient` / `MultiChartDataWorkerClient`:
  DuckDB 操作を専用 Web Worker に offload。ingest と
  rendering 中も複数チャートが non-blocking。
- 高度な用途向け subpath export: `uneri/align` (時系列 alignment
  ヘルパー), `uneri/multiWorkerClient` (multi-chart worker クライアント),
  `uneri/workerProtocol` (worker メッセージ型)。
- `@duckdb/duckdb-wasm` 1.32.0 によるブラウザ内 OLAP + `uplot` 1.6
  rendering 層。React ≥ 18 を peer dependency として要求。

### `starlight-rustdoc` (npm)

- Astro / Starlight 統合。`cargo rustdoc --output-format json` 出力を
  自動生成 Markdown API ページに変換。
- category 別 (Traits, Structs, Enums, Functions, Type Aliases, Constants)
  の item ごとページ生成。Starlight sidebar への自動組み込み。
- cross-crate link resolver: page registry を維持し、locale-agnostic の
  相対 URL を出力。同じ生成 Markdown が `/en/...` と `/ja/...` で
  locale 別再 rendering なしに動作。
- multi-crate サポート: Cargo feature フラグ, default-features toggle,
  Rust ツールチェーン選択 (デフォルト `nightly`)。
- source link 統合 (`repository` + branch を生成ページに埋め込み) と
  プレビュービルド用の skip 可能な生成。
- `sidebar: false` オプション: sidebar entry の自動追加を無効化し、
  sidebar 構造の完全な手動制御を可能にする。
- 汎用・再利用可能 — このリポジトリに同梱されているが orts 固有ではない。
  Starlight `config:setup` hook plugin として呼び出されるため、
  任意の Astro / Starlight サイトで Rust crate のドキュメントに採用可能。

