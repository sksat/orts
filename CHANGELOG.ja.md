# Changelog (日本語)

[Keep a Changelog](https://keepachangelog.com/ja/1.1.0/) に緩く準拠。
[Semantic Versioning](https://semver.org/) で versioning。

orts は マルチパッケージ workspace (crates.io Rust crate + npm package)。
全パッケージを同一バージョンでリリースし、セクションはパッケージ別に分割。

## [Unreleased]

### `orts` (Rust, crates.io)

#### Added
- 地上局コンタクトウィンドウ検出 (`visibility` module): `GroundStation`
  (WGS-84 位置 + 仰角マスク)、`ContactWindow` (補間した AOS/LOS、最大仰角、
  span クリップフラグ)、純粋な `PassTracker` ステートマシン、frame-aware な
  `VisibilityMonitor<F: EarthFixedTransform>` (ECI サンプルを地上局ごとの
  topocentric look angle に変換)。([#112](https://github.com/sksat/orts/pull/112))
- `IndependentGroup::propagate_to_with(t_target, observer)` — 受理された全
  積分ステップで `FnMut(&SatId, f64, &State)` observer を呼びながら伝播。
  integrator 解像度で状態をサンプリングできる。`propagate_to` は no-op
  observer で委譲し、軌道はビット単位で不変。([#112](https://github.com/sksat/orts/pull/112))
- node messaging 層 (`plugin::message`、"msg-io"): FSW のコマンド & テレメトリ
  用。`Message`、`NodeId` (`Ground` / `Satellite(u32)`)、`Payload`、
  `NamedValue`、`Value` (`Boolean`/`Integer`/`Number`/`Text`/`Bytes`) を
  `orts::plugin` から再エクスポート。([#58](https://github.com/sksat/orts/pull/58))
- `PluginController` の transport hook (既定 no-op、WASM backend が実装):
  msg-io の `deliver` / `take_outbound`、raw byte stream 用 stream-io の
  `stream_deliver` / `stream_take` / `stream_close`。([#58](https://github.com/sksat/orts/pull/58), [#84](https://github.com/sksat/orts/pull/84))
- WIT v0 plugin interface に msg-io / stream-io チャネルを追加。([#58](https://github.com/sksat/orts/pull/58), [#84](https://github.com/sksat/orts/pull/84))
- `perturbations::ZonalGravity<F: EarthRotationPole = SimpleEci>` — zonal 調和項
  (J2 と任意の J3 / J4) を `GravityField` でなく極を意識した摂動 `Model` として
  扱う。フレームの Z 軸を極と決め打ちせず、中心天体の自転極まわりで評価する
  (`ZonalGravity::new(mu, r_body, j2, j3, j4)`)。`SimpleEci` では極が `+Z` なので
  従来の frame-Z 式と数値的に同一、`Gcrs` では IAU 2006 CIP なので J2 が地球の
  真の極まわりで評価される (2024 epoch で GCRS Z 軸から ~0.1°)。
  `setup::build_orbital_system` / `build_spacecraft_dynamics` は
  `PointMass` + `ZonalGravity` を合成する (これらが構築する `SimpleEci` 系では
  振る舞い保存)。drag なしの J2 + 第三体 ISS クロスバリデーションでは、Orekit に
  対する `Gcrs` の最終位置誤差が 1 桁以上改善する。([#194](https://github.com/sksat/orts/pull/194), [#204](https://github.com/sksat/orts/pull/204))
- `PanelSrp` を frame-generic 化。`Model` 実装が `SimpleEci` 固定でなく
  `F: EphemerisFrameBridge` 境界になり、`GCRS → F` のエフェメリス回転を定義する
  任意のフレームで姿勢依存の panel SRP が使える。GCRS 整列の
  `SimpleEci` / `Gcrs` ではビット単位で同一。([#238](https://github.com/sksat/orts/pull/238))

#### Changed
- `StateEffector` を frame-generic 化 — `StateEffector<S, F: frame::Eci =
  SimpleEci>` で `ExternalLoads<F>` を返す (`Model<S, F>` と同様)。effector は
  host の慣性 frame で荷重を生成するようになった。既定の `F` により既存の
  `StateEffector<S>` 実装はそのままコンパイル可能。([#148](https://github.com/sksat/orts/pull/148))
- `ThirdBodyGravity` と `SolarRadiationPressure` を、あらゆる `Eci` フレームに
  blanket 実装するのをやめて frame-correct にした。`Model` 実装を
  `arika::earth::transform::EphemerisFrameBridge` 境界にし、解析 (GCRS) の
  太陽 / 月エフェメリスを積分フレームへ回転してから衛星状態と差分を取る。
  `SimpleEci` / `Gcrs` では数値不変 (identity 回転)、`Cirs` では precession /
  nutation 回転が適用され、`EphemerisFrameBridge` 実装の無いフレーム (例 `Teme`)
  は黙った誤フレーム混在でなくコンパイルエラーになる。([#193](https://github.com/sksat/orts/pull/193), [#237](https://github.com/sksat/orts/pull/237), [#191](https://github.com/sksat/orts/issues/191))
- 大気抵抗の co-rotation 速度 `Ω × r` を、フレームの `+Z` でなく中心天体の真の
  自転軸まわりで取る。地球では軸を積分フレーム上の
  `EarthRotationPole::earth_pole` から得る — `SimpleEci` は `+Z` (従来の
  `[0, 0, ω] × r` とビット単位で同一)、`Gcrs` は IAU 2006 CIP となり、`+Z` 仮定に
  よる ~0.1–0.3° のずれが解消する。`|Ω|` の LOD 変動は依然未実装。地球以外の
  中心天体ではフレーム Z 軸とその天体の `omega_body` を維持し、その天体の実際の
  自転軸の向きはモデル化しない (既知の制限)。([#209](https://github.com/sksat/orts/pull/209), [#210](https://github.com/sksat/orts/issues/210))
- **BREAKING**: `perturbations::BodyPositionFn` — したがって
  `ThirdBodyGravity::custom` — が `&Epoch` (`Epoch<Utc>`) でなく `&Epoch<Tdb>` を
  取る。天体エフェメリスは力学時の量であるため。力モデルの呼び出し境界で
  `.to_tdb()` 変換する。カスタムエフェメリスのクロージャは型を変更する必要が
  ある。([#222](https://github.com/sksat/orts/pull/222))
- **BREAKING**: `magnetic::field_inertial`、`visibility::VisibilityMonitor`、
  `perturbations::AtmosphericDrag` のフレーム capability 境界が
  `orts::environment::EarthFrameBridge` から `arika::earth::EarthFixedTransform`
  になった。関連アイテム (`Fixed`、`EopStorage`、`to_geodetic`、
  `fixed_to_inertial`) は不変なので、下流は import パスの変更のみで済む。([#213](https://github.com/sksat/orts/pull/213))

#### Fixed
- `SpacecraftDynamics` の不正な frame 再タグを除去。`ExternalLoads<SimpleEci>`
  とタグ付けされた effector 荷重を変換なしで host frame `F` に貼り替えており、
  `F != SimpleEci` (例: `Gcrs`) で座標を黙って誤ラベルしていた。出荷済み
  effector が torque のみのため潜在的だったが、並進 effector では誤りとなる。([#148](https://github.com/sksat/orts/pull/148), [#103](https://github.com/sksat/orts/issues/103))
- 力モデルに渡す絶対エポックを、一様 SI タイムライン上の `epoch_0 + t`
  (`Epoch::add_si_seconds`) にした (`OrbitalSystem`、`SpacecraftDynamics`、
  `AttitudeSystem`、`DecoupledAttitudeSystem`、`AugmentedAttitudeSystem`、
  `VisibilityMonitor::update`)。従来の leap 非対応な `add_seconds` は `t` を UTC
  カレンダー上で進めていたため、leap second を跨ぐステップで物理時間が 1 秒余分に
  進み、エフェメリス・地球回転・コンタクトウィンドウの幾何を誤った瞬間で評価して
  いた。([#215](https://github.com/sksat/orts/pull/215))

#### Removed
- **BREAKING**: `orts::tle` module を削除。TLE パースは `arika::tle`
  (共有 `arika::elements::Sgp4Elements` へデコード) に移管。`orts::tle` を使う
  下流コードは `arika` へ移行が必要。([#87](https://github.com/sksat/orts/pull/87))
- **BREAKING**: `orts::environment` module を削除。内容は `arika` へ移設・改名
  した: `EarthFrameBridge` → `arika::earth::EarthFixedTransform`、
  `EarthPoleBridge` → `arika::earth::EarthRotationPole`、`PositionEop` /
  `GcrsEopStorage` → `arika::earth::eop::{PositionEop, GcrsEopStorage}`
  (`arika::earth` からも再エクスポート)。`orts` 側に互換の再エクスポートは
  残していない。なお `arika` の `PositionEop` は `Send + Sync` を要求しない —
  スレッド安全性は boxed な `GcrsEopStorage` が担う。([#213](https://github.com/sksat/orts/pull/213))
- **BREAKING**: `orbital::gravity::ZonalHarmonics` を削除。`GravityField` は真に
  frame-invariant な `PointMass` のみになった。扁平率は摂動 `Model` として扱う —
  `OrbitalSystem::new(mu, Box::new(ZonalHarmonics { r_body, j2, j3, j4 }))` を
  `OrbitalSystem::new(mu, Box::new(PointMass)).with_model(ZonalGravity::new(mu, r_body, j2, j3, j4))`
  に置き換える (`mu` を摂動側に明示的に渡す点に注意)。([#204](https://github.com/sksat/orts/pull/204))

### `orts-cli` (Rust, crates.io, binary)

#### Added
- `orts run` の地上局コンタクトウィンドウ出力: `[[ground_station]]`
  (`name`、`latitude_deg`、`longitude_deg`、`altitude_km`、`min_elevation_deg`)
  で局を宣言。検出したウィンドウを AOS 順に stderr へ出力 (UTC 時刻、
  sim-time オフセット、最大仰角。`*` は sim span でクリップされたウィンドウ)。
  Earth 中心・エポック必須。([#112](https://github.com/sksat/orts/pull/112))
- コンタクトウィンドウは `--output-interval` でなく integrator 解像度
  (受理ステップごと / 制御 tick ごと) でサンプリング。検出が出力間引きに
  依存しなくなった (1 サンプル間隔より短いパスは依然取りこぼし得る)。([#112](https://github.com/sksat/orts/pull/112))
- `--omm <file>` で CCSDS OMM 入力 (JSON / KVN / XML、`-` で stdin) を
  `arika::elements::parse` でパース。TLE ペイロードは拒否し `--tle` を案内。([#87](https://github.com/sksat/orts/pull/87))
- `orts serve` の `--stream-stdio SAT/STREAM` — 宣言済み stream-io stream の
  1 本を kble-socket protocol で stdin/stdout に接続し、orts を kble の
  `exec:` plug として実行可能にする。その stream の WebSocket endpoint は
  HTTP 409 を返し、stdio peer が閉じるとサーバを停止。([#114](https://github.com/sksat/orts/pull/114))
- `orts serve` の stream-io kble ブリッジ: 宣言済み各 stream を
  `/stream/{sat}/{stream}` のバイナリ WebSocket endpoint として realtime
  loop で駆動。未宣言ペアは HTTP 404。stream は config の `streams` で
  衛星ごとに宣言。([#106](https://github.com/sksat/orts/pull/106))
- config 駆動のコマンドタイムライン (FSW コマンド & テレメトリ):
  `[[command]]` entry (`t` sim-time、`sat`、`kind`、任意の型付き `args`)。
  host がスケジュール tick で決定論的に配送 (`orts run` のみ)。([#58](https://github.com/sksat/orts/pull/58))
- WebSocket protocol の TypeScript 型を `ts-rs` で Rust 型から生成
  (protocol enum、`SimConfig`、`SatelliteInfo` 等に `#[derive(TS)]`)。
  `cargo test -p orts-cli` 実行時に viewer へ出力し、ドリフトすると CI が落ちる。([#95](https://github.com/sksat/orts/pull/95))
- `orts run --json` で機械可読な実行サマリを stdout に出力 — `status`、解決済み
  の `simulation` パラメータ、各衛星の `samples` 数と `final` 位置/速度、出力
  `artifacts`。診断ログは stderr に残すので、stdout はちょうど 1 つの機械可読
  ドキュメントを運ぶ。スクリプトや orts を駆動する coding agent 向け。stdout が
  JSON を運ぶため、シミュレーションデータはファイルへ出力する必要があり、
  `--json` とデータの stdout 出力の併用は拒否する。([#214](https://github.com/sksat/orts/pull/214))
- `orts run --output -` でシミュレーションデータを stdout に出力。`-` を正準の
  stdout sentinel とし、従来の `stdout` キーワードは alias として残す。([#214](https://github.com/sksat/orts/pull/214))
- `orts config` サブコマンド群。config ファイルを正準入力として誘導する:
  `config example [--format toml|json|yaml]` で編集可能な example config を出力、
  `config validate <path> [--json]` で config を検証し結果を報告(人間向けは
  stderr、`--json` で機械可読 verdict を stdout に。exit 0=valid / 2=invalid)。
  `--sat` の help もこの経路を案内するようにした。([#216](https://github.com/sksat/orts/pull/216))
- `orts --help`(および `-h`)の末尾に、主要ワークフロー(ファイルへの run、
  `--json` 実行サマリ、`config example`/`validate`、`serve`)のコピペ可能な実例を
  追加。よく使う・agent に関係する経路を端末内で発見できるようにした。([#217](https://github.com/sksat/orts/pull/217))

#### Changed
- `--tle` を再び TLE 専用 (2LE/3LE、`-` で stdin) とし、新規 `--omm` と
  対にした。要素セットのパースは削除した `orts::tle` でなく
  `arika::tle` / `arika::omm` を使用 (従来は `--tle` が OMM も自動受理)。([#87](https://github.com/sksat/orts/pull/87))
- 軌道ソースの排他フラグは、片方を黙って優先するのでなくエラーにする:
  `--sat` と `--tle` / `--omm` / `--tle-line1` / `--tle-line2` / `--norad-id`、
  `--tle` と `--omm`、ファイルソースとインライン `--tle-line1` / `--tle-line2`
  は併用不可。`--tle-line1` / `--tle-line2` は両方指定が必須。([#87](https://github.com/sksat/orts/pull/87))
- TLE epoch の day-of-year を (閏考慮の) 年日数で検証。不正値はそのまま別の
  年に繰り上がらず拒否される。([#87](https://github.com/sksat/orts/pull/87))
- **BREAKING**: config キーの alias を、キーごとに正準の 1 綴りへ集約した。
  `[[command]]`、`[[ground_station]]`、`[satellites.magnetorquers]`、
  `[satellites.thruster]` のみが受理され、従来の `[[commands]]`、
  `[[ground_stations]]`、`[satellites.mtq]`、`[satellites.thrusters]` は削除。
  うち `mtq` / `thrusters` はリリース済み 0.2.0 の config での正準綴りだった。
  未知のキーは拒否されず無視されるため、古い綴りの config はその節を**黙って**
  失う — キーを改名すること。([#200](https://github.com/sksat/orts/pull/200))
- **BREAKING**: TLE / OMM 軌道の初期状態を、接触ケプラー要素として読むのでなく
  実際の SGP4 伝播で求める。要素セットを `arika` の SGP4 で評価エポックまで伝播し、
  得られた TEME 状態を積分フレームへ回転する (`FrameTransform<Teme, SimpleEci>`)。
  従来の二体変換はエポックで数十 km 誤っていたため、`--tle` / `--omm` /
  `--norad-id` / `[satellites.orbit] type = "tle"|"norad"` の軌道は変化する。
  あわせて `--epoch` (または config の `epoch`) 省略時、最初の衛星が TLE / OMM
  軌道ならシミュレーションエポックが**その要素セットのエポック**(tsince = 0) を
  既定にする (従来は「現在時刻」)。要素セットが複数ある場合、tsince = 0 は最初の
  1 つだけで、残りは共有エポックから外挿される。円軌道は影響を受けない。([#241](https://github.com/sksat/orts/pull/241))
- **BREAKING**: TLE / OMM 軌道と地球以外の中心天体の組み合わせを事前に拒否する
  (SGP4 / TEME は地球中心・WGS72)。3 経路すべてで有効: `orts run` / `orts serve`
  の起動時、WebSocket の `StartSimulation` config (panic でなくクライアントへ
  エラー)、動的な `add_satellite`。従来は地球基準の SGP4 状態を他天体の μ・半径・
  摂動セットで積分していた。([#241](https://github.com/sksat/orts/pull/241))
- `orts serve` のモデル別加速度内訳 (WebSocket `State` メッセージの
  `accelerations` map) に `zonal_gravity` エントリが加わり、`gravity` は点質量項
  のみになった。扁平率が重力場から独立した摂動モデルへ移ったため。合計加速度は
  不変。([#194](https://github.com/sksat/orts/pull/194))
- 実行中の `orts serve` セッションに追加した衛星は、初期状態を
  `epoch + current_t` で評価する。TLE / OMM は `t = 0` でなく参加した瞬間まで
  伝播される。不正な要素セットはサーバをクラッシュさせずエラーとして報告する。([#241](https://github.com/sksat/orts/pull/241))
- config 検証を厳格化し、config を読む全経路 (`orts run`、
  `orts serve --config`、`orts config validate`) でシミュレーション開始前に走る
  ようにした。未知の `body`、不正な `epoch`、不正なインライン
  `[satellites.orbit] type = "tle"` は後段の panic でなく明確なエラーになる。([#216](https://github.com/sksat/orts/pull/216))
- `orts run` は矛盾する stdout 要求を (長時間になりうる) シミュレーション実行前に
  拒否し、これら使用法エラーには exit status 2 を返す。`--format rrd` の stdout
  出力も対象で、従来はシミュレーション完走後に exit 1 していた。([#214](https://github.com/sksat/orts/pull/214))

#### Fixed
- `orts run --format csv --output <path>` が CSV を `<path>` に書き込むように
  なった。従来は `--format csv` の実行が `--output` に関わらず常に stdout へ
  出力し、指定パスを黙って無視していた。([#214](https://github.com/sksat/orts/pull/214))
- `--config` ファイルで起動した `orts serve` は `[[command]]` タイムラインを
  含む config を明確なエラーで拒否する (コマンドタイムラインは `orts run`
  のみ)。従来は黙って破棄していた。([#58](https://github.com/sksat/orts/pull/58))
- シミュレーション時刻を一様 SI タイムライン (`Epoch::add_si_seconds`) 上で
  進めるようにした。leap second を跨ぐ実行でも、力モデル・センサ・plugin tick に
  渡すエポックが 1 秒余分にずれない。([#215](https://github.com/sksat/orts/pull/215))

### `orts-plugin-sdk` (Rust, crates.io)

#### Added
- `msg-io` node messaging 層 (FSW コマンド & テレメトリ、将来の衛星間通信用):
  WIT `interface msg-io` (`recv-batch` / `send-message`) が、論理 `node-id`
  (`ground` / `satellite(u32)`) で宛先指定した型付き `payload` の datagram を
  `tick-io` 制御プレーンとは別に運ぶ。SDK は `msg` module (`recv_batch`、
  `recv_all`、`send`、`send_to`、`key_value`、`get`、`get_text`) を追加し
  `Message` / `Outbound` / `NodeId` / `Payload` / `Value` / `NamedValue` を
  再エクスポート。([#58](https://github.com/sksat/orts/pull/58))
- `stream-io` raw byte-stream チャネル (kble 仮想ハーネス統合用): WIT
  `interface stream-io` (名前付き stream の `read` / `write`)。orts は単なる
  byte 導管で、framing は FSW + kble パイプライン側に委ねる。SDK は `stream`
  module (`read`、`write`、`read_bytes`) を追加し `StreamRead` / `StreamError`
  を再エクスポート。([#84](https://github.com/sksat/orts/pull/84))
- example FSW に detumble→nadir モード遷移ガードを追加 (`commandable-mode-ff`、
  `commandable-mode-rr`)。([#58](https://github.com/sksat/orts/pull/58))

#### Changed
- **BREAKING**: `world plugin` が `msg-io` と `stream-io` を追加で import する。
  変更は純粋に追加的 (既存の interface / import / export / record の削除・改変
  なし) のため、`orts_plugin!` の callback 型 guest は影響なし。手書きの
  `impl Guest` guest は binding を再生成し新規 host import をリンクする必要がある。([#58](https://github.com/sksat/orts/pull/58), [#84](https://github.com/sksat/orts/pull/84))

### `arika` (Rust, crates.io)

#### Added
- 要素セットのパース ([#87](https://github.com/sksat/orts/pull/87), [#230](https://github.com/sksat/orts/pull/230), [#245](https://github.com/sksat/orts/pull/245))。共有の
  no-alloc な `elements::Sgp4Elements` (平均要素セット: カタログ番号、UTC epoch、
  6 個の SGP4 平均要素、B\* drag。角度は rad、平均運動は rad/s) を**検証付き型**に。
  `Sgp4Elements::try_new` / `TryFrom<Sgp4ElementsFields>` で構築し、各フィールド
  有限・mean motion 正・eccentricity ∈ [0,1) を強制(違反は `ElementsError`)。
  フィールドは `fields()` で読み、表示用ヘルパ `semi_major_axis(mu)` と `period()`
  を持つ。テキストパーサは `elements::ParsedElementSet` (要素 + 所有する
  `OBJECT_NAME` / `OBJECT_ID` 識別子) を返し、検証に失敗した要素セットは reject
  する。形式判定する `elements::parse` がそれらに振り分ける。
  - `tle` — NORAD TLE / 2LE / 3LE パーサ (`tle::parse`) が `ParsedElementSet` を生成。
    Alpha-5 英数字カタログ番号と `OBJECT_ID` 正規化に対応。
  - `omm::json` / `omm::kvn` / `omm::xml` — JSON / KVN / XML 各シリアライズの
    CCSDS OMM パーサ。JSON は単一オブジェクトまたは 1 要素配列 (CelesTrak の
    単一衛星 GP) と Space-Track の文字列エンコード数値を受理。
  - `elements::detect` + `elements::parse` — 形式判定 (`elements::Format`) と、TLE / OMM-JSON /
    OMM-KVN / OMM-XML を自動判定して振り分ける BOM 許容の統一エントリ。
- optional な `sgp4` feature による SGP4 / SDP4 伝播
  (`sgp4::Sgp4Propagator`): `from_elements` が epoch の `Constants` を呼び出し
  間で再利用し、`propagate_minutes_since_epoch` / `propagate(Epoch<Utc>)` が
  TEME の `(Vec3<Teme>, Vec3<Teme>)` 状態 (km / km·s) または `Sgp4Error`
  (`Initialization` / `Diverged`。非有限の要求時刻も含む) を返す。`sgp4` crate を AFSPC compatibility mode (WGS72) でラップ。依存は
  `libm` のみで引くため no_std-no-alloc ビルドでも動作。Vallado 検証ベクタの
  near-earth (SGP4) と deep-space (SDP4) で検証済み。([#235](https://github.com/sksat/orts/pull/235))
- TEME↔GCRS / TEME↔SimpleEci フレーム回転。SGP4 の `Vec3<Teme>` 状態を積分
  フレームへ変換する。`earth::fk5` に equinox ベースの IAU-76/FK5 換算
  (IAU-76 precession、フル 106 項 IAU-80 nutation、mean obliquity、equation of
  the equinoxes、GMST 1982。各々対応する ERFA ルーチンを再現)、`earth::teme` に
  `Rotation<Teme, Gcrs>::teme_to_gcrs`、`Rotation<Teme, SimpleEci>::teme_to_simple_eci`
  (`R3(GMST−ERA)` の z 回転)、`FrameTransform<Teme, Gcrs>` / `FrameTransform<Teme, SimpleEci>`
  の状態(位置+速度)変換(ω=0)。
  J2000→GCRS の frame bias (~数十 mas、LEO で ≈ サブメートル) は無視。
  ERFA(component, 1e-11)と Orekit (authoritative TEME, ~0.8 m)で交差検証。([#240](https://github.com/sksat/orts/pull/240))
- `kepler` module (`orts` から `arika` へ移管): `KeplerianElements`
  (`from_state_vector` / `to_state_vector` / `period` / `energy`) と anomaly
  変換群 (`solve_kepler_equation`、`mean_to_true_anomaly` 等)。公開
  `arika::kepler` surface となり、`orts::orbital::kepler` が再エクスポート。([#87](https://github.com/sksat/orts/pull/87))
- `frame::Teme` marker — True Equator, Mean Equinox (SGP4 / TLE 出力 frame)。([#87](https://github.com/sksat/orts/pull/87))
- `earth::topocentric` — 地上局の look angle: `TopocentricSite<F: Ecef>`
  (WGS-84 `Geodetic` から構築し局所 ENU 基底を事前計算) と `LookAngles`
  (方位 / 仰角 / slant range)。`look_angles(target)` で算出。([#112](https://github.com/sksat/orts/pull/112))
- GPS Time を first-class な時刻 scale に: `epoch::Gps` marker と `Epoch<Gps>`
  (`from_jd_gps`)、変換エッジ (`Epoch::<Utc>::to_gps`、`Epoch::<Tai>::to_gps`、
  `Epoch::<Gps>::to_tai` / `to_utc`)。leap second を持たない固定の TAI − 19 s。
  GNSS の week / seconds-of-week は生の整数でなく型で表す: `GpsWeek`
  (1024 週ロールオーバのない連続カウント)、10-bit の航法メッセージ week を解決
  する `GpsWeek::from_broadcast(raw10, reference)`、範囲検査付き
  `SecondsOfWeek`、`Epoch::<Gps>::from_week_seconds` / `to_week_seconds`、
  定数 `GPS_EPOCH_JD`。([#192](https://github.com/sksat/orts/pull/192))
- `epoch::FixedOffsetFromTai` — TAI からのオフセットが固定 SI 秒である scale の
  capability trait (`Tai` 0、`Tt` +32.184、`Gps` −19)。`SECONDS_AFTER_TAI` が
  対応する変換エッジの単一の真実になる。UTC (leap second による区分)、TDB
  (周期項)、UT1 (EOP 依存) は意図的に実装しないので、「TAI からのオフセットが
  定数」は規約でなく型レベルの事実になった。([#192](https://github.com/sksat/orts/pull/192))
- `Epoch::duration_since(&earlier)` — 共有 TAI タイムライン上で測る厳密な SI 秒
  間隔。leap second を跨いでも、scale が異なっても正しい (`earlier` は別 scale で
  よい)。([#205](https://github.com/sksat/orts/pull/205))
- `epoch::TwoPartJd` と sealed な `epoch::JdRepr` — ユリウス日を `hi + lo` の
  2 部分 (SOFA 形式) で保持し、日内小数に完全な mantissa を残す。
  `Epoch::jd_parts()` が単一 `f64` へ潰さずに `(hi, lo)` を渡す。([#205](https://github.com/sksat/orts/pull/205), [#208](https://github.com/sksat/orts/pull/208))
- 精度 tier を型に: `Epoch<S, P: Precision>`。sealed な tier は `Precise`
  (既定、2 部分 JD、16 バイト、sub-nanosecond) と `Coarse` (単一 `f64`、8 バイト、
  数十 µs。RAM と cycle が厳しい wasm / no_std ターゲット向け)。
  `Epoch::to_precision::<Q>()` で変換し `precision_name()` で参照する。tier は
  monomorphize され、混在には明示変換が必要。`Epoch` / `Epoch<S>` は従来どおり
  `Epoch<Utc, Precise>` を意味する。([#208](https://github.com/sksat/orts/pull/208))
- `earth::transform` — フレームごとの地球姿勢 capability trait 群。
  `orts::environment` から移設・改名した: `EarthRotationPole` (`earth_pole`。
  zonal 重力と大気の co-rotation が必要とする最小の capability) と
  `EarthFixedTransform` (対になる `Fixed: Ecef` フレーム、`to_geodetic`、
  `fixed_to_inertial`、状態変換の factory)。`SimpleEci` (ERA のみの Z 回転、極は
  `+Z`) と `Gcrs` (IAU 2006 CIO チェーン、極は model CIP) に実装。支える
  `earth::eop::PositionEop` bound と型消去された `earth::eop::GcrsEopStorage` も
  一緒に移設。([#213](https://github.com/sksat/orts/pull/213))
- `frame::FrameTransform<From, To>` — `Rotation` の運動学的な相棒。向きに加えて
  `From` に対する `To` の角速度を持ち、transport theorem で速度と状態全体を
  変換する (`transform_position`、`transform_velocity`、`transform_state`、
  `inverse`、`angular_velocity_in_from`)。地球 ECI↔ECEF の実体は
  `EarthFixedTransform::inertial_to_fixed_transform` /
  `fixed_to_inertial_transform` から得る。ω は `OMEGA · earth_pole` で地球スピンの
  transport のみ (IAU 2006 の precession / nutation / polar motion の rate と LOD
  補正は省略)。([#219](https://github.com/sksat/orts/pull/219))
- `earth::transform::EphemerisFrameBridge` — `GCRS → F` の回転。third-body / SRP
  の力モデルが、GCRS 整列フレームでしか正しくない生ベクトルを消費するのでなく、
  解析 (Meeus) エフェメリスを積分フレームで表現できるようにする。`Gcrs` と
  `SimpleEci` は identity、`Cirs` は EOP 不要の IAU 2006 model 回転
  (`Rotation::<Gcrs, Cirs>::iau2006_model`)。impl の無いフレームはこれらの力と
  組み合わせられないので、新しい of-date フレームは `GCRS → F` 回転を明示する
  必要がある。([#237](https://github.com/sksat/orts/pull/237))

#### Changed
- `Epoch::from_iso8601` が ordinal / day-of-year 形式
  (`YYYY-DDDTHH:MM:SS`、CCSDS OMM で使用) も受理し、末尾の `Z` が任意になった。
  厳密な緩和で、従来受理した入力は引き続きパース可能。([#87](https://github.com/sksat/orts/pull/87))
- **BREAKING**: `Epoch<S>` は内部で scale 固有のユリウス日でなく **正準 TAI の
  瞬間**を保持する。scale 変換 (`to_tai`、`to_tt`、`to_tdb`、`to_gps` 等) は同一の
  瞬間への非破壊な再タグになり、leap second テーブル・固定 TAI オフセット・TDB の
  周期項は構築時 (`from_*`) と読み出し時 (`jd()`) にのみ適用される。
  `from_jd_*(x).jd() == x` の往復は維持され `TimeScale` は sealed のままだが、
  `Epoch` はもはや透過的な JD ラッパではない — `jd()` は保持フィールドでなく
  scale ごとの読み出しであり、異なる経路で構築した同一の物理的瞬間が等しく
  比較されるようになった。([#205](https://github.com/sksat/orts/pull/205))
- **BREAKING**: 解析エフェメリスは `&Epoch<Utc>` を受けて内部変換するのをやめ、
  `&Epoch<Tdb>` を要求する — `sun::sun_direction_eci`、`sun_position_eci`、
  `sun_distance_km`、`equation_of_time`、`sun_direction_from_body`、
  `sun_distance_from_body`、`moon::moon_position_eci`、`planets::obliquity`、
  `planets::heliocentric_position_ecliptic`。これら Meeus モデルが定義される
  力学時が型に現れる。呼び出し側が境界で `.to_tdb()` すれば数値は同一。UTC で
  索引する `MoonEphemeris` trait は意図的に `&Epoch<Utc>` を維持。([#222](https://github.com/sksat/orts/pull/222))

#### Fixed
- `Epoch::<Utc>::add_si_seconds` が一様 TAI タイムライン上の厳密な SI 加算に
  なった。従来の UTC を反復する実装は、結果が挿入された leap second の内側に
  落ちる場合に最大 1 秒ずれていた。`add_si_seconds(dt)` の後に
  `duration_since` で `dt` が復元されることを 2017-01-01 の leap で回帰テストに
  固定した。([#205](https://github.com/sksat/orts/pull/205))

#### Removed
- **BREAKING**: `epoch::Ut1` scale marker を削除。UT1 は地球の自転で実現される
  **観測量**(EOP) であって TAI からのデータ不要なオフセットではないため、
  `Epoch<S>` が保持するようになった正準 TAI タイムラインを共有できない。
  独立した `epoch::Ut1Epoch` (`from_jd_ut1`、`jd`、`era`) に移し、
  `Epoch::<Utc>::to_ut1(eop)` / `to_ut1_naive()` からのみ到達する。
  `&Epoch<Ut1>` を取っていたシグネチャは `&Ut1Epoch` を取る:
  `Rotation::<SimpleEci, SimpleEcef>::from_ut1`、
  `Rotation::<SimpleEcef, SimpleEci>::from_ut1`、
  `Rotation::<Cirs, Tirs>::from_era`、`Rotation::<Gcrs, Itrs>::iau2006_full`。([#205](https://github.com/sksat/orts/pull/205))

### `utsuroi` (Rust, crates.io)

#### Added
- `IntegrationError` が `core::error::Error` を実装 (手書き、`thiserror` 不使用、
  `no_std` でも動作)。`?` 連鎖や `Box<dyn Error>` に乗るようになった。([#147](https://github.com/sksat/orts/pull/147))

### `tobari` (Rust, crates.io)

#### Changed
- CSSI 宇宙天気ダウンロードの feature `fetch` を `fetch-cssi` にリネーム。
  `fetch-<source>` 規約(`fetch-igrf`、arika の `fetch-horizons`)に揃えた。
  `fetch` は全 `fetch-*` 源を束ねる傘 feature として存続するため、
  `features = ["fetch"]` は引き続きビルド可能(加えて `fetch-igrf` も有効化)。([#150](https://github.com/sksat/orts/pull/150))
- **BREAKING**: `HarrisPriester::with_sun_direction_fn` が
  `fn(&Epoch) -> Vec3<Gcrs>` (すなわち `Epoch<Utc>`) でなく
  `fn(&Epoch<Tdb>) -> Vec3<Gcrs>` を取る。TDB エポックを要求するようになった
  `arika` の解析エフェメリスに合わせたもの。モデル自身は UTC 入力を保ち、呼び出し
  境界で `.to_tdb()` 変換する。太陽方向 hook を差し替えている場合は関数の型を
  変更する必要がある。([#222](https://github.com/sksat/orts/pull/222))

#### Fixed
- 太陽エフェメリスの量を正しい時刻 scale で評価するようにした。Harris-Priester の
  密度バルジ頂点は太陽方向を問う前に UTC エポックを TDB へ変換し、
  `nrlmsise00::geo::local_solar_time` も均時差で同様にする。従来は UTC エポックを
  TDB 引数のエフェメリスへそのまま渡していた (~69 秒の scale 誤差)。([#222](https://github.com/sksat/orts/pull/222))

### `viewer`

#### Added
- 新しい `./lib` エントリ (`viewer/src/lib`) による組み込み可能な viewer
  ライブラリ。同梱 SPA だけでなく任意の React + `@react-three/fiber` アプリに
  orbit viewer を組み込める。レイヤ化 API:
  - `OrbitViewer` — オールインワン: 自前のサイズ付き `<div>` + `<Canvas>` を
    描画。`centralBody` と `SatelliteState[]` で駆動。
  - `OrbitScene` — 自分の `<Canvas>` 内にマウントする scene graph
    (bring-your-own Canvas)。エクスポートされた `SCENE_UP` で初期化。
  - viewer 自身のアプリも公開 `OrbitScene` API 上に構築 (dogfooding)。
    ライブラリとアプリが乖離しない。
  ([#89](https://github.com/sksat/orts/pull/89), [#175](https://github.com/sksat/orts/pull/175), [#176](https://github.com/sksat/orts/pull/176))
- shadcn registry としての配布 (`registry.json`、item `orbit-viewer`):
  component とその primitive を `shadcn add` で consumer アプリに取り込める。
  registry item を導入して描画するスタンドアロン consumer example
  (`viewer/examples/orbit-viewer/`) を同梱。([#168](https://github.com/sksat/orts/pull/168), [#169](https://github.com/sksat/orts/pull/169))
- 拡張可能な中心天体: `bodies` prop (`BodyDefinitions`) でカスタム定義を渡し、
  組み込みの `DEFAULT_BODIES` (Earth / Moon / Sun / Mars) に重ねる。
  `BodyDefinition` / `BodyDefinitions` / `BodyTexture` / `DEFAULT_BODIES` を
  エクスポート。([#164](https://github.com/sksat/orts/pull/164))
- 注入可能な arika WASM: `initArika({ wasmUrl? })` / `isArikaReady()` を
  エクスポート。embedder が module を事前ロードしたり外部 `.wasm` URL を
  指定できる。arika WASM は独自 workspace package (`arika-wasm`) に切り出し、
  名前で import する (registry 配布に必要)。([#159](https://github.com/sksat/orts/pull/159), [#167](https://github.com/sksat/orts/pull/167))
- 公開 `TrailBuffer` streaming primitive (`TrailBuffer` + `TrailBufferLike`):
  呼び出し側が bounded な trail buffer を所有し React の外で mutate でき
  (`SatelliteState.trailBuffer`)、scene が毎フレーム読むため streaming した
  点が React 再レンダーなしで GPU に届く。`toTrailBuffer` /
  `trailPointToOrbitPoint` と `OrbitPoint` / `TrailPoint` 型をエクスポート。([#176](https://github.com/sksat/orts/pull/176))
- `SatelliteState` の衛星ごと表示プロパティ: `color`、`name`、`markerShape`、
  `trailDisplay` (`visibleCount` / `drawStart`、playback スクラブ用)、
  および衛星ごとの `time` (凍結 / スクラブした衛星のマーカーをその body-fixed
  trail に整合させる)。([#89](https://github.com/sksat/orts/pull/89), [#176](https://github.com/sksat/orts/pull/176))
- 衛星を trail だけでなく現在位置から描画 — 位置のみ (trail なし) の衛星も
  マーカーを表示する。([#89](https://github.com/sksat/orts/pull/89))
- 選択可能なマーカー形状 (`MarkerShape`: `"sphere"` | `"axes-cube"`)。
  3D モデルなしで姿勢を示す非球の XYZ 姿勢キューブを含む。衛星ごと / scene
  全体で解決でき、シミュレーションが wire 越しに宣言可能 (viewer 上書き可)。([#158](https://github.com/sksat/orts/pull/158))
- 衛星中心 frame が要求された向きを尊重: star-fixed `inertial` (軸が共回転
  しない) または `localOrbital` (LVLH)。従来は衛星中心ビューが常に LVLH に
  収束していた。([#111](https://github.com/sksat/orts/pull/111), [#90](https://github.com/sksat/orts/issues/90))

#### Changed
- `./lib` の公開 barrel は意図的に絞っている: Three.js / r3f の構成要素と内部
  frame 配線はエクスポートしない。公開 surface は `OrbitViewer`、`OrbitScene`、
  `TrailBuffer` / `TrailBufferLike`、`toTrailBuffer` / `trailPointToOrbitPoint`、
  `initArika` / `isArikaReady`、`SCENE_UP`、`DEFAULT_BODIES`、
  `DEFAULT_VIEWER_FRAME` と対応する型。([#177](https://github.com/sksat/orts/pull/177))
- DuckDB-wasm アセットを viewer 側で self-host (Vite `?url` import を uneri の
  `initDuckDB({ bundles })` に渡す)。jsDelivr CDN への runtime 依存を排除。([#171](https://github.com/sksat/orts/pull/171))
- Earth 固有の描画 (day/night terminator、大気、Earth の自転) を「night
  texture を持つか」でなく `earth` body id に限定。カスタム天体は汎用の
  textured-sphere 経路で描画。([#164](https://github.com/sksat/orts/pull/164))
- 中心天体に解決可能な半径がない場合、`OrbitScene` / `OrbitViewer` は半径 1 で
  黙ってフォールバックして scene scale を狂わせるのでなく、明確なエラーを出す。([#164](https://github.com/sksat/orts/pull/164))
- arika WASM は `epochJd` が与えられた時のみロード。epoch なしの embedder は
  init コストを払わない (固定の Sun 方向、天体回転なし)。([#89](https://github.com/sksat/orts/pull/89))
- WS protocol 型は `ts-rs` 生成 binding になった (`orts-cli` 参照)。手書きの
  wire 型を置き換え、`satellite_added` variant を追加。([#95](https://github.com/sksat/orts/pull/95))
- **BREAKING**: `ts-rs` の wire binding が正準化された config キーに追従した —
  `SatelliteConfig.mtq` → `magnetorquers` (`start_simulation` の `SimConfig`
  payload と、flatten された `add_satellite` メッセージの両方)、
  `SimConfig.commands` → `command`、`SimConfig.ground_stations` →
  `ground_station`。古いキーは拒否されず無視されるため、`mtq` を送り続ける WS
  クライアントはエラーなしに magnetorquer 設定を失う。([#200](https://github.com/sksat/orts/pull/200))

#### Fixed
- static deploy での既定 WebSocket URL を、`window.location` から到達不能な
  host を導出するのでなく `ws://localhost:9001/ws` にフォールバック。([#143](https://github.com/sksat/orts/pull/143))
- static deployment で高解像度天体テクスチャを復元 (サーバからのみ取得、
  off-thread デコード、bounded な upgrade retry、in-flight guard)。([#88](https://github.com/sksat/orts/pull/88), [#113](https://github.com/sksat/orts/pull/113), [#105](https://github.com/sksat/orts/issues/105))
- LVLH (衛星中心) の中心天体の向きを修正。Earth (ERA) と非 Earth
  (`body_orientation` + pole) で経路を分離。([#51](https://github.com/sksat/orts/pull/51))
- quaternion slerp が `qw` だけでなく完全な quaternion (qw/qx/qy/qz 全て) を
  guard し、NaN はそのまま通す。scene の per-render アロケーションを削除。([#172](https://github.com/sksat/orts/pull/172))
- trail buffer の mutation を commit phase で適用、trail buffer reset 時に
  `satellites[]` を再構築、body-fixed マーカーで衛星ごとの位置時刻を保持、
  `SatelliteState.color` を尊重、file / RRD adapter は restart 時にクリーンに
  リセットし fatal な worker error で破棄。([#89](https://github.com/sksat/orts/pull/89), [#107](https://github.com/sksat/orts/pull/107), [#108](https://github.com/sksat/orts/pull/108), [#176](https://github.com/sksat/orts/pull/176))

#### Performance
- trail なしの衛星は trail buffer 確保と毎フレームの trail 処理を丸ごとスキップ。([#107](https://github.com/sksat/orts/pull/107))

### `uneri` (npm: `@sksat/uneri`)

#### Added
- `initDuckDB` が DuckDB-wasm の worker / wasm を、jsDelivr CDN でなく
  呼び出し側が注入する self-host bundle URL からロード可能に。新しい
  `DuckDBInitOptions` (`bundles?`、`fallbackToJsDelivr?`) と `DuckDBBundleUrls`
  型、純粋関数 `resolveBundleSource(options?)` を追加。uneri は bundler 中立の
  まま、アプリ側が URL を解決して渡す。([#171](https://github.com/sksat/orts/pull/171))
- 堅牢な init: `initDuckDB` は linear backoff でリトライし、死んだ worker は
  `error` listener で即 fail (ハングしない)、terminal failure 後はキャッシュ
  された reject promise を破棄して次回呼び出しでリトライする。([#76](https://github.com/sksat/orts/pull/76), [#70](https://github.com/sksat/orts/issues/70))

#### Changed
- 引数なしの `initDuckDB()` の既定動作は不変 — 引き続き jsDelivr CDN から
  bundle を取得するため既存 consumer はそのまま動く。self-host は
  `options.bundles` で opt-in。([#171](https://github.com/sksat/orts/pull/171))

#### Fixed
- init 時の worker 404 / "invalid URL": bundle URL を `initDuckDB` 内で worker
  origin に対して絶対化する。DuckDB が worker を `blob:` URL から生成するため、
  root-relative パスでは解決できないことへの対処。([#171](https://github.com/sksat/orts/pull/171))

### Docs

#### Added
- ドキュメントサイトに `llms.txt` / `llms-full.txt` / `llms-small.txt` を生成
  (`starlight-llms-txt`)。coding agent や LLM ツールが docs を取り込めるようにした
  — 例: <https://sksat.github.io/orts/llms.txt> を agent に渡す。`llms-full.txt`
  は全文、`llms-small.txt` は自動生成 API リファレンスを除いた要約版。([#225](https://github.com/sksat/orts/pull/225))

#### Changed
- ドキュメントサイトを Astro 7 + Starlight 0.40 (および `@astrojs/react` 6) で
  動かすようにした。sidebar の `autogenerate` グループは Starlight 0.40 の
  ネスト schema へ移行 (そのままでは 0.40 が受理しない)。自動生成の rustdoc /
  typedoc API セクションは従来どおり折りたたみのまま。([#126](https://github.com/sksat/orts/pull/126), [#180](https://github.com/sksat/orts/pull/180), [#250](https://github.com/sksat/orts/pull/250))
- docs サイトを CI で gate するようにした。ドキュメントに影響する PR で Astro /
  Starlight のフルビルドを走らせ (従来は main への push のみで、ビルド退行が
  マージ後に判明していた)、サイトの `.astro` / `.ts` ソースを `astro check` で
  型検査する (`astro build` では検査されない)。([#180](https://github.com/sksat/orts/pull/180), [#233](https://github.com/sksat/orts/pull/233))

### Dependencies

- Rust toolchain → 1.96.1。
- 新規 Rust 依存: `sgp4` 2.4。`arika` の optional な `sgp4` feature 経由で、
  `libm` のみを引くため no_std / no-alloc tier も引き続きビルドできる。
  `orts-cli` が有効化する。
- Rust: `wasmtime` / `wasmtime-wasi` 44 (security)、`rerun` 0.33、
  `tokio-tungstenite` 0.29、`nalgebra` 0.35、`tokio` 1.52、`axum` 0.8.9、
  `kble-socket` 0.5 (E2E job の `kble` CLI も 0.5)、`rand` 0.10 /
  `rand_distr` 0.6、`wit-bindgen` 0.58 (`orts-plugin-sdk` の binding 生成)、
  `clap` 4.6、`thiserror` 2.0、`toml` 1.1。
- `notalawyer` 0.3 — 埋め込む third-party ライセンス NOTICE を、`cargo about`
  バイナリでなく cargo-about **ライブラリ**(`orts-cli` の build-dependency)で
  生成するようにした。CI でのバイナリ install と cross ビルドイメージへの
  埋め込みが不要になった。
- npm: `vite` 8、`@vitejs/plugin-react` 6、React monorepo、`ws` 8.21
  (security)、`mermaid` 11.16 (security)、Astro 7 (`@astrojs/starlight` 0.40 +
  `@astrojs/react` 6。Astro 7.1.0 は security リリース)、TypeScript 6、
  Biome 2.5、`vite-plugin-dts` 5、`three` 0.184、docs の `llms.txt` 生成に使う
  `starlight-llms-txt` 0.10。
- ツール: pnpm 11 — workspace は `allowBuilds` を明示的に宣言し (pnpm 11 は
  さもなければ install を拒否する)、`minimumReleaseAge` の供給網フロアが
  `--frozen-lockfile` install でも強制される。加えて Node.js 24 と
  wasi-sdk 33 (`nos3-adcs` example のビルド)。

## [0.2.0](https://github.com/sksat/orts/releases/tag/v0.2.0) - 2026-04-20

リリースブログ記事: [orts: 人工衛星シミュレーションプラットフォームを作りました](https://sksat.hatenablog.com/entry/orts-release)

- `ARCHITECTURE.md` (EN/JA) を新規追加。言語間の自動リンク書き換え機構付き
- orts logo kit を docs / viewer / README に統合
- ブランド名表記を `Orts` → `orts` (小文字) でリポジトリ全体で統一
- Notable dependency updates:
  - Rust: `nalgebra` 0.34、`clap` 4.6、`criterion` 0.8、`ureq` 3.3、
    `toml` 1.1、`proptest` 1.11、`rand` 0.9.4 (security)
  - npm: `@astrojs/starlight` 0.38.3、`@biomejs/biome` 2.4、
    `happy-dom` 20.8.9 (security, dev only)

### `orts` (Rust, crates.io)

#### Added
- SRP と sun sensor が `arika::eclipse` を利用し、円錐半影
  (conical penumbra) を考慮した連続照度スケーリング / 日食検出に対応
- Per-device アクチュエータコマンド
  - MTQ・RW を個別デバイスリストとして管理し、デバイス単位で指令を送信
- マルチインスタンスセンサ: sensor を `Vec` ベースに変更し任意の個数に対応
- RW モーター一次遅れ (first-order lag) モデル
- RW 速度指令 / トルク指令バリアントと `MtqCommand` variant
- 非直交 RW/MTQ レイアウト向けの擬似逆行列トルク・ダイポール配分
- Fine/Coarse バリアント付きサンセンサモデル
- Controlled simulation の姿勢・コマンド・テレメトリログ
  - 動的 CSV カラム生成
- `ThrusterSpec` 導入 — host スケジュール `Thruster` と plugin 指令型
  `ThrusterAssembly` で物理パラメータを共有 (MTQ の Core+Assembly パターンを踏襲)

#### Changed
- **BREAKING**: B-dot detumble コントローラを `BdotDetumbler` → `BdotCross`
  に rename。`BdotFiniteDiff` との命名一貫性を取り、dB/dt 推定手法
  (cross-product `-ω × B` vs finite difference) の違いを明示
- アクチュエータ telemetry をアクチュエータ種別横断で統一的に構造化
- `orts convert` を姿勢・コマンド・テレメトリを含むフルデータ出力に拡張
- CSV metadata・satellite 出力を `SimMetadata::write_csv_header` /
  `write_satellite_csv` に統一

### `orts-cli` (Rust, crates.io, binary)

#### Added
- WASM plugin からの thruster スロットル指令 (`[0,1]` per-device) を
  controlled simulation loop に配線 (Phase P4)

#### Changed
- **BREAKING**: `orts run` は orbit 指定が必須に。`--sat` / `--tle` /
  `--norad-id` / `--config` / CWD の `orts.toml` のいずれも無い場合は
  エラーを返す。従来の「無指定なら 400km 円軌道」はサイレントすぎるため廃止
- **BREAKING**: `--altitude` フラグを削除。軌道指定は
  `--sat "altitude=400,inclination=51.6"` または config file で
  明示する形に統一
- `orts run` が CWD の `orts.toml` を自動検出 (解決順序:
  `--config` > CLI orbit args > `orts.toml` > エラー)

### `orts-plugin-sdk` (Rust, crates.io)

#### Added
- `no_std` サポート
  - 標準ライブラリなし (allocator 不要) でコンパイル可能
  - オプションの `alloc` feature flag で `no_std` 下でのヒープ使用に対応
- WIT plugin interface に thruster throttle 指令 (`[0,1]` per-device) を追加。
  全 example plugin で新コマンドフィールドに対応
- 新規 example: `nos3-adcs` — NOS3 `generic_adcs` WASM plugin (SILS デモ)
  - 全モードテスト、IGRF 統合、可視化スクリプト、CI workflow
- 新規 example: `constellation-phasing` — コンステレーション位相制御デモ
- 新規 example: `transfer-burn-with-tcm` — 軌道遷移 + trajectory
  correction maneuver デモ

#### Changed
- **BREAKING**: WIT v0 の sensor / actuator / command 構造を再編。既存
  plugin はバインディング再生成と tick handler の更新が必要:
  - sensor: `option<T>` → `list<T>` (magnetometer / gyroscope /
    star-tracker / sun-sensor の multi-instance 化)
  - actuator: `ActuatorState` → `ActuatorTelemetry` (RW は
    `RwTelemetry` として構造化)
  - command: `commanded-magnetic-moment` / `commanded-rw-torque` を
    `mtq-command` / `rw-command` variant に置換、`thruster-command`
    variant を追加
  - sun sensor: `sun-fine-output.direction` を `option` 化
    (total eclipse で `None`)、fine / coarse variant を新設
- example plugin を `plugin-sdk/examples/` workspace に移動
- WIT bindings 生成を `wit_bindgen::generate!()` ベースに移行
  (従来の `cargo component` 依存を軽量化)
- `bdot-finite-diff` example をより長時間のシミュレーション +
  複数モデル比較構成に刷新

### `arika` (Rust, crates.io)

#### Added
- `eclipse` モジュール — cylindrical (binary) と conical
  (Montenbruck & Gill penumbra) の 2 種類の shadow モデルを提供する
  汎用 illumination API (observer / light / occulter)
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

### Docs

#### Added
- Starlight docs サイトに LaTeX 数式レンダリング
  (`remark-math` + `rehype-katex`)
- Starlight docs サイトに Mermaid 図レンダリング (`astro-mermaid`)
- example README を YAML frontmatter で自動発見し、ドキュメントページとして展開

#### Changed
- example の制御則記述を LaTeX 数式に移行
- crate の sidebar グループを既定で展開、API エントリのみ折りたたんだ状態に
  して navigation を効率化

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

### `viewer`

- React + `@react-three/fiber` (Three.js) + Vite によるリアルタイム 3D
  軌道ビューア。`orts-cli` バイナリに同梱し `http://localhost:9001` で配信。
  standalone SPA としても deploy 済。
- 3D シーン: 汎用 `CelestialBody` component によるテクスチャ付き中心天体
  (Earth/Moon/Sun/Mars)。地球の day/night terminator と大気散乱の
  カスタム GLSL shader、orbit-controls カメラ。
- 衛星ごとの可視化: 3D 軌跡 trail、表示スケール設定可能な 3D 衛星モデル
  (衛星中心ビューでは実スケール表示)、姿勢 quaternion による body-frame
  の姿勢軸表示。
- 参照フレーム選択: 中心天体中心の inertial (ECI) / body-fixed (ECEF) 表示、
  または衛星を中心にしてその局所軌道 (LVLH) フレームで追尾。ECI ↔ ECEF
  変換は `arika` WASM ビルドでブラウザ内実行。
- データソース: CSV と `.rrd` 軌道ファイルの読込 (`.rrd` は `rrd-wasm` で
  ブラウザ内 decode)、および `orts serve` から telemetry を streaming する
  ライブ WebSocket モード (`useWebSocket`)。単一・多数の衛星に対応。
- ブラウザ内シミュレーション制御: シミュレーション parameter を設定し、
  実行中の `orts serve` シミュレーションを UI から pause / resume / terminate。
- replay / playback: `useRealtimePlayback` hook が時刻ベースの軌道再生を駆動。
  軌跡の漸進描画と `PlaybackBar` scrubber (再生 / 一時停止 / シーク)。
- ブラウザ内解析: DuckDB-wasm + uPlot 時系列チャート (`uneri` ベース)。
  ドラッグズーム、マルチ衛星 series。

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

