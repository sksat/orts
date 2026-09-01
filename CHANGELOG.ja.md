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

#### Changed
- `SatelliteParams` が `SpacecraftShape` を optional で持ち、
  `build_spacecraft_dynamics` がそれを見て等方面の `SolarRadiationPressure` /
  `AtmosphericDrag` の代わりに `PanelSrp` / `PanelDrag` を install するように
  した。機体の外形は一つなので、パネルは片方ではなく両方の力を担う。
  `build_orbital_system` はどちらも install しない (パネルの力は姿勢を要求する)。
  `SurfacePanel` に `with_cp_offset` を追加した。パネルの力を姿勢外乱にするのは
  この offset である。([#386](https://github.com/sksat/orts/pull/386))
- `SurfacePanel::back_face` で薄板の反対面を作れるようにした。法線を反転し、面積 /
  `cd` / 圧力中心は引き継ぎ、光学係数は引数で受ける (パドルの両面は性質が違う)。
  パネルは片面で、両モデルとも太陽や流れと逆を向いた面を落とすので、板を 1 枚として
  書くとその衛星が取る姿勢の半分で力がゼロになり、重心から外れた圧力中心が作る
  トルクも出ない。閉じた形状の面には使わない (反対側は既に別のパネルである)。([#395](https://github.com/sksat/orts/pull/395))
- 外乱トルクの登録を `orts::setup` に集約し、どれを解くかを `SatelliteParams` の
  `DisturbanceTorques` で選ぶようにした。`build_spacecraft_dynamics` がそれを見て
  gravity gradient トルクを install する。`build_orbital_system` は install しない
  (軌道のみの系にはトルクが作用する姿勢が無く、`torque_body` を捨てる)。
  呼び出し側は環境モデルを登録しなくなった。CLI はこのトルクを 2 つの entry point
  で同一行に書いていて、両者が食い違わない保証が無かった。actuator (RW, MTQ,
  thruster) の登録は呼び出し側に残る。搭載する actuator は機体のハードウェア記述で
  決まる。([#382](https://github.com/sksat/orts/pull/382))
- `SurfacePanel` が lumped な `cr` の代わりに `optics: PanelOptics { specular,
  diffuse }` を持つようになった。吸収率は `1 - specular - diffuse` として導出する。
  単一係数は face-on の SRP 力の大きさを決めるだけで、斜入射での向きが決まらない
  ── 下の平板 SRP 修正が必要とするのはその向きである。`PanelOptics` の field は
  private で、`new` の検証を struct literal で迂回できない: specular が 1 を超えると
  吸収率が負になり、力の太陽方向成分が太陽側を向く。`SpacecraftShape::Sphere` は
  `cr` を維持する。等方面では lumped 係数がモデルの定義そのもので、specular / diffuse
  の項が依存する入射角が存在しない。`SurfacePanel::at_com` は optics を必須引数に取り、
  `SpacecraftShape::cube` は `cr` の代わりに optics を取る。SRP 力が黙って変わるのでは
  なくコンパイルエラーとして出るようにするため: `Cr = 1.5` は単一の `(ρ_s, ρ_d)`
  に対応せず、力の向きがこの分解に依存するようになったので、どの既定値も face-on 以外
  で旧振る舞いを再現しない。面が本当に不明なら `PanelOptics::absorber()` を渡す。([#377](https://github.com/sksat/orts/pull/377))
- **BREAKING**: `EntityStore::timelines` の型が `Vec<TimeIndex>` から
  `TimelineColumn` になり、列は自分が覆う論理行を持つようになった。追加は論理行を
  明示して検査する `ComponentColumn::push_at` を通す。`push` と `scalars_per_row`、
  両 column type の `data` / `rows` field は crate 内限定 — map と data が食い違った列は、
  値を別の行の時刻で報告する。読み出しは列が `scalars` と `scalars_per_row()`、軸が
  `times`、行単位では `get_row` が格納 index、`at_logical_row` が
  論理行。([#375](https://github.com/sksat/orts/issues/375))
- `StateEffector` を frame-generic 化 — `StateEffector<S, F: frame::Eci =
  SimpleEci>` で `ExternalLoads<F>` を返す (`Model<S, F>` と同様)。effector は
  host の慣性 frame で荷重を生成するようになった。既定の `F` により既存の
  `StateEffector<S>` 実装はそのままコンパイル可能。([#148](https://github.com/sksat/orts/pull/148))
- `arika` の暦フレーム修正に伴い、太陽・月に依存する結果 (SRP、第三体重力、日陰幾何、
  sun sensor、Harris-Priester の密度 bulge) が動く。暦が mean equinox of date ではなく
  J2000 の方向を返すようになったためで、2024 年で 0.335°。Orekit との一致はその分改善し、
  GEO 3 日の third-body oracle は 218 m → 0.33 m、短い Harris-Priester oracle 3 件は
  20-40% 改善する。([#359](https://github.com/sksat/orts/pull/359))

#### Fixed
- 一部の step でしか logging されなかった component が、logging された時刻を保つ
  ようになった。`log_temporal` は行数の比較で新しい行かどうかを決めていたため、
  短い列が**先頭の**行に並んでいた: step 5 から attitude を出すと step 5〜9 が
  t=0〜4 として書かれていた。行は `TimePoint` で識別するようになり、
  `ComponentColumn` と新しい `TimelineColumn` がそれぞれ自分の覆う論理行を持つ
  (毎 step logging される列は `RowMap::Dense` なので通常ケースのコストはゼロ)。
  「この step に値が無い」は `log_temporal` を呼ばないことで表現でき、
  API 追加は不要。([#375](https://github.com/sksat/orts/issues/375))
- entity の他の行が持たない軸を名指しする `TimePoint`、および軸を別の順序で
  組んだ `TimePoint` が、行を分けたり誤配置したりしなくなった。同じ軸を同じ index で
  名指す 2 点は、組んだ順序に関係なく 1 行。`with_*` は軸の index を置き換える
  (2 つ目を append しない)。`+0.0` と `-0.0` は同一の瞬間で、`NaN` 時刻でも
  component ごとでなく step ごとに 1 行になる。([#375](https://github.com/sksat/orts/issues/375))
- .rrd の loader 2 つが、scalar 列を recording 自身の時刻 index で結合するようになった。
  一部の時刻にしか現れない component が、後の値を前の行にずらすことがなくなる。
  `load_rrd_data` は `orts replay` と `orts serve` の history 読み戻し、
  `load_as_recording` は `orts convert` が通る。後者では疎な列が 2 つの時刻の値の
  混合として CSV に出ていた (`Position3D` が `[100.0, 201.0, 0.0]` になり、ある時刻の
  `x` と別の時刻の `y` が組になっていた)。`EntityStore` は entity の全列で 1 つの
  timeline を共有するので、行の key も entity で 1 つに決め、position と velocity に
  合わせる。どちらかが揃わない時刻は行にしない。一部の時刻にしか記録されていない
  component はゼロ埋めせず落とす。ゼロは下流で実測値と区別できないため。timeline を
  持たない static field と、独自の時刻を持つ子 entity は、上位 entity の行を増やさなく
  なった。独自の名前の timeline で index された recording も、その timeline で結合する。
  `sim_time` と `step` は orts が書く名前で、それ以外を timeline なしとして扱うと列の
  位置で結合していた。その名前は recording 全体で 1 つで、`sim_time` と `step` の代わりでなく
  それらと並んで行を識別する。軸が 2 つあれば別の次元なので、`frame` の 1 は
  `iteration` の 1 でも `step` の 1 でもなく、同じ `sim_time` で `frame` が違う 2 行は
  別の瞬間。この PR が直すブラウザ側の
  decoder と同じ欠陥で、3 つは独立に
  decode するため 3 つとも抱えていた。([#366](https://github.com/sksat/orts/pull/366))
- `PanelSrp` が平板ごとの反射力を出すようになった。per-panel の力は
  `-P·Cr·A·cosθ·ŝ` で常に反太陽方向だった。平板では鏡面反射と拡散再放射が panel
  normal 方向の力を作るが、その項が無く、力が
  `F = -P·A·cosθ·[(α + ρ_d)·ŝ + 2·(ρ_s·cosθ + ρ_d/3)·n̂]` に従うのは黒体の場合だけ
  だった。SRP トルクは大きさだけでなく向きも誤っていた: 圧力中心が ŝ–n̂ 平面の外に
  あるとき `r × ŝ` と `r × n̂` は別方向を向くので、`Cr` をどう選んでも正しい答えには
  ならない。太陽電池パドル相当 (ρ_s ≈ 0.2, ρ_d ≈ 0.1) では 45° 入射で欠けていた項が
  力の ~30% を占める。model の導入時から存在し、0.2.0 も該当する。
  図解: [光子の行き先](docs/src/assets/srp-flat-panel/photon-fates.svg)、
  [2 成分の合成](docs/src/assets/srp-flat-panel/force-composition.svg)、
  [トルクの向き](docs/src/assets/srp-flat-panel/torque-direction.svg)。
  ([#377](https://github.com/sksat/orts/pull/377))
- `AttitudeState::q_dot` が、和を計算した後でなく積を作る前に角速度を半分にする
  ようになった。結果が有限な入力で overflow しなくなる: `q = [0, 1/√2, 1/√2, 0]`、
  `ω = [1.4e308, 1.4e308, 0]` の `q̇.w` は約 -9.9e307 だが、途中の和が -1.98e308 に
  達し、後から 0.5 を掛けても無限は戻らなかった。([#343](https://github.com/sksat/orts/pull/343))
- `AttitudeState::is_finite` が、成分の有限性だけでなく四元数ノルムが正かつ有限で
  あることを要求するようになった。成分が 1e157 程度なら各々有限でも二乗和は無限に
  なり、その四元数は姿勢を指さない (`orientation()` はそのノルムで割る)。`project` は
  それをもっともらしいゼロに潰さず放置するので、拒否するのは integrator の有限性
  検査しかない。従来はそれを生んだステップが成功として報告され、その状態が
  センサと plugin controller に渡っていた。([#343](https://github.com/sksat/orts/pull/343))
- `SpacecraftDynamics` の不正な frame 再タグを除去。`ExternalLoads<SimpleEci>`
  とタグ付けされた effector 荷重を変換なしで host frame `F` に貼り替えており、
  `F != SimpleEci` (例: `Gcrs`) で座標を黙って誤ラベルしていた。出荷済み
  effector が torque のみのため潜在的だったが、並進 effector では誤りとなる。([#148](https://github.com/sksat/orts/pull/148), [#103](https://github.com/sksat/orts/issues/103))
- 負の `sim_time` が .rrd に届くようになった。`save_as_rrd` は timeline を
  `set_duration_secs` で設定していたが、これは符号なしの `std::time::Duration`
  を経由するので負値を拒否し、直前の行の timestamp をそのまま残していた。
  `-30, -20, -10, 0, 10` は `0, 0, 0, 0, 10` として書かれていた。([#379](https://github.com/sksat/orts/pull/379))

#### Performance
- `save_as_rrd` が .rrd を値ごとの `rec.log()` でなく
  `RecordingStream::send_columns` で列単位に書くようになった。呼び出し回数は
  行数でなく field 数に比例する: 2500 行 13 field の segment で 32,500 回が 13 回。
  行単位のループは `HistoryBuffer::flush` 224ms のうち呼び出しスレッドの 165ms を
  占めており、writer スレッドの Arrow IPC + LZ4 は 55ms、665KB のファイル書き込みは
  0.2ms だった。batcher を迂回して chunk の境界が呼び出し側の責任になるため、
  8192 行で分割する。([#379](https://github.com/sksat/orts/pull/379))

#### Removed
- **BREAKING**: `orts::tle` module を削除。TLE パースは `arika::tle`
  (共有 `arika::elements::Sgp4Elements` へデコード) に移管。`orts::tle` を使う
  下流コードは `arika` へ移行が必要。([#87](https://github.com/sksat/orts/pull/87))

### `orts-cli` (Rust, crates.io, binary)

#### Added
- `[[satellites.panels]]` で衛星に平板の外形を与えられるようにした
  (`area`, `normal`, `cd`, `specular`, `diffuse`, `cp_offset`, `two_sided`
  ([#399](https://github.com/sksat/orts/pull/399))、`back` ([#395](https://github.com/sksat/orts/pull/395)))。
  パネルは片面なので、薄板 (太陽電池パドル) には裏面を書く。表裏が同じなら
  `two_sided = true`、光学的に違うなら `back` に違う分の反射率を書く (空の
  `back = {}` は `two_sided = true` と同じ)。面積 / `cd` / 圧力中心はどちらでも
  同じ板として共有する。`two_sided = false` と `back` の同時指定は矛盾なので
  reject する。パネルを書くと
  SRP と大気抵抗が姿勢依存になり、圧力中心が重心から外れていればそれが姿勢外乱に
  なる。等方面の `srp_area_to_mass` / `ballistic_coeff` では表せない部分である。
  パネルは `attitude` を要求し、等方面のパラメータとの同時指定は reject する
  (同じ力を二通りに述べることになる)。パネルが 0 枚のリストも reject する。
  等方面で表すならキーを書かない。0 枚の外形は何も述べていない。パネルの加速度は wire 上では力の名前
  (`drag`, `srp`) で報告する。あのチャネルはモデル単位ではなく物理力単位だから
  である。どのモデルかは `perturbations` が述べ、姿勢を持つ衛星についてはこれを
  軌道のみの系を組み直して読むのではなく、実際に伝播する dynamics から取るように
  した。([#386](https://github.com/sksat/orts/pull/386))
- `[satellites.disturbances]` で衛星がモデル化する環境外乱トルクを選べるように
  した。`gravity_gradient` の既定は true で、姿勢伝播が従来から持っていた振る舞い。
  `[satellites.attitude]` の中ではなく sibling の table にしたのは、前者が姿勢の
  状態と機体特性を述べる場所であり、どの環境モデルを解くかは別の関心事だから。
  `attitude` 不在での指定は reject する。([#382](https://github.com/sksat/orts/pull/382))
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
- `orts run` の downlink log レコードの level を info/debug から debug/trace へ
  下げた。衛星 × outbound message × control tick ごとに出るため、logger が
  実際に動く状態では info だと fleet 規模の実行で他の診断が読めなくなり、
  積分ループ内で stderr のロックを取る書き込みが増える。 ([#390](https://github.com/sksat/orts/pull/390))
- `--tle` を再び TLE 専用 (2LE/3LE、`-` で stdin) とし、新規 `--omm` と
  対にした。要素セットのパースは削除した `orts::tle` でなく
  `arika::tle` / `arika::omm` を使用 (従来は `--tle` が OMM も自動受理)。([#87](https://github.com/sksat/orts/pull/87))
- 軌道ソースの排他フラグは、片方を黙って優先するのでなくエラーにする:
  `--sat` と `--tle` / `--omm` / `--tle-line1` / `--tle-line2` / `--norad-id`、
  `--tle` と `--omm`、ファイルソースとインライン `--tle-line1` / `--tle-line2`
  は併用不可。`--tle-line1` / `--tle-line2` は両方指定が必須。([#87](https://github.com/sksat/orts/pull/87))
- TLE epoch の day-of-year を (閏考慮の) 年日数で検証。不正値はそのまま別の
  年に繰り上がらず拒否される。([#87](https://github.com/sksat/orts/pull/87))
- config ファイル、`orts config validate`、WebSocket の `start_simulation`
  payload は、シミュレーションが実行できない入力を別の値に読み替えず拒否する:
  未知の `[integrator] type` / `atmosphere` (従来は dp45 / exponential
  モデル)、id が 1 つの recording entity を指す 2 機 (id の文字列でなく entity で
  比較するので、recording と同じく `a` と `/a` は衝突する。従来は recording
  entity と CSV section を共有し、id 文字列まで同じなら `[[command]]` の宛先も
  共有していた)、
  どの剛体もとりえない attitude ブロック (正定値でない、または主慣性モーメントで
  `I1 + I2 >= I3` を破る慣性テンソル、`mass <= 0`、正規化できない
  `initial_quaternion`)。config が受理する `integrator` / `atmosphere` の綴りは、
  対応する CLI フラグが受理する集合と厳密に一致する。

  どのフィールドも読まないキーは、拒否せずキー名を warning として表示する。実行は
  続くので新しい `orts` 向けに書いた config も古い `orts` で実行でき、`duraton = 100`
  が黙って 1 周期分実行されることはなくなった。`orts config validate` は `warnings`
  にキーのパスを出力し、`run` と `serve` は `log::warn!` で報告する。server は client の
  `start_simulation` や `add_satellite` に含まれるものを表示する。`type` タグ付きの
  block (`[satellites.orbit]`、`[satellites.controller]`、reaction wheel、磁気トルカ)
  だけは拒否する。`serde_ignored` は internally tagged enum の内部を見られず報告
  できないため、`inclinaton = 51.6` が無視されると軌道が赤道面のままになる。特異な
  慣性テンソルは従来
  `SpacecraftDynamics::new` で panic し、`orts serve --config` では spawn された
  manager task の中で起きるため、server は listen したままシミュレーションも
  client への error も無い状態になっていた。([#351](https://github.com/sksat/orts/pull/351))

#### Fixed
- `orts` が診断ログを stderr に出力するようになった。logger を初期化していな
  かったため `log::` の呼び出しは全て破棄されており、stream-io stdio plug の
  displaced、stream の socket error、`serve` 中のシミュレーション停止、
  WASM plugin が WIT `host-env.log` 経由で出力した行が、どれも表示されなかった。
  `.rrd` を書く際に rerun の crate が出す診断も同じ出力に含まれる。level は
  `RUST_LOG` で選び、既定は `warn,orts=info` (orts は info、依存は warn)。
  `NO_COLOR` 指定時と stderr が terminal でない場合は装飾を付けない。どちらも
  `orts --help` に記載がある。stdout は従来どおりコマンドの出力 (CSV、`--json`
  サマリ、`serve --stream-stdio` の protocol) だけ。 ([#390](https://github.com/sksat/orts/pull/390))
- `tle` / `norad` 軌道を Earth 以外の中心天体で使う config を、
  `orts config validate` と `orts serve --config` が拒否する。SGP4 は Earth 専用で、
  `SimParams::from_config` はこの規則に panic 経由で到達していたため、config は valid
  と判定された上で `orts run --config` が panic していた。([#351](https://github.com/sksat/orts/pull/351))
- どの mode でも実行できない fleet を `orts config validate` と
  `orts serve --config` が拒否する。`[satellites.attitude]` や
  `[satellites.controller]` を一部の衛星にだけ書いた config と、全衛星に controller
  があって attitude がどこにもない config。engine は元からこれらを拒否していたが、
  `orts serve` は engine を spawn した manager task 内で構築するため、config は valid
  と判定され banner も表示された上で、指定した config が実行されないまま server が
  idle 状態で待機していた。`serve` は listening と表示せずエラー終了する。([#351](https://github.com/sksat/orts/pull/351))
- WebSocket の `add_satellite` が、実行中の fleet の衛星と同じ entity path
  (`/world/sat/<id>`) になる id を拒否する。従来は同じ path の 2 機を受理し、
  `[[command]]` は後から追加した方にしか配送されなかった。`id` 省略時の
  `sat-<現在の機数>` も同様に衝突する。([#351](https://github.com/sksat/orts/pull/351))
- `orts serve` が、`ClientMessage` に deserialize できなかったメッセージに
  `{"type":"error"}` を返す。従来は error を破棄していたため、client は応答が
  返らないまま待ち続けた。deserialize が失敗するのは `type` タグ付き block に未知の
  キーがある場合。([#351](https://github.com/sksat/orts/pull/351))
- `orts serve` が `Server listening` / `WebSocket endpoint` の banner を、
  `--config` ファイルを受理した後にだけ出すようになった。先に出していたため、
  banner を起動完了として待つ呼び出し側 (`cli/tests/ws_e2e.rs`、Playwright の
  spec) には、拒否された config が error message ではなく接続失敗として
  届いていた。([#351](https://github.com/sksat/orts/pull/351))
- `orts run --format csv --output <path>` が CSV を `<path>` に書き込むように
  なった。従来は `--format csv` の実行が `--output` に関わらず常に stdout へ
  出力し、指定パスを黙って無視していた。([#214](https://github.com/sksat/orts/pull/214))
- `--config` ファイルで起動した `orts serve` は `[[command]]` タイムラインを
  含む config を明確なエラーで拒否する (コマンドタイムラインは `orts run`
  のみ)。従来は黙って破棄していた。([#58](https://github.com/sksat/orts/pull/58))
- `orts run` が `[satellites.attitude]` を honor する。全衛星に姿勢設定がある
  fleet は `orts serve` と同じ spacecraft dynamics (姿勢状態 + coupled gravity
  gradient) で伝播され、CSV に四元数と機体系角速度の列が増える。従来は全衛星が
  controller を持たない限り軌道だけを伝播し、姿勢・アクチュエータ・センサ設定を
  警告なしに捨てていた (README のクイックスタート設定もその一つ)。モード判定
  (orbit-only / spacecraft / controlled) は `orts run`、`ServeEngine::build`、
  serve の WebSocket config 検証が共有する 1 箇所に集約したので、同じ config が
  エントリポイントによって姿勢を伝播したりしなかったりすることはなくなった。
  ([#335](https://github.com/sksat/orts/pull/335))
- 実行モードが act できない設定は黙って捨てず報告する: `orts run` は配送先の
  controller がない `[[command]]` タイムラインと、宣言された stream-io の
  `streams` (run のループには pump する transport がない) を拒否する。両
  エントリポイントは controller 設定の混在を拒否する (制御ループは fleet 全体を
  回すか全く回さないかなので、混在させると controller が回らない)。WebSocket の
  `start_simulation` も `orts serve --config` と同じ `[[command]]` 拒否を適用
  する。実行中の orbit-only シミュレーションへの `controller` 付き衛星の動的
  追加を拒否する。`sensors` / `reaction_wheels` / `magnetorquers` / `thruster`
  が controller なしで宣言された場合は stderr と `orts run --json` の
  `warnings` に警告を出す。([#335](https://github.com/sksat/orts/pull/335))
- config 単体から「ダイナミクスを構築できない・開始できない」と分かる姿勢設定を、
  config を読むすべての経路で拒否するようになった。`orts config validate` も含む (従来は valid と報告した
  設定を `run` と `serve` が拒否していた)。検査は `AttitudeConfig` に置き、
  範囲チェックだけでは通ってしまうものを塞いだ: `NaN` はあらゆる比較が false に
  なるので `NaN` の質量が `mass <= 0.0` をすり抜けていた。慣性テンソルは
  「ダイナミクスが取る逆行列が存在し `I·I⁻¹ ≈ E` を満たすか」で判定する。
  行列式の大きさによる閾値ではこれを判定できず、条件数 1 の `[1e-11; 3]` を拒否し、
  逆行列が有限のゼロ行列になって(あらゆるトルクに角加速度ゼロで応える)
  `[1e154; 3]` を受理していた。torque-free な t=0 の角加速度が有限で
  あることを要求する。軌道を必要とするもの (シミュレーションが実際に
  始める微分に含まれる gravity-gradient torque) は判定の範囲外で、そちらは最初の
  ステップで run を止める。`orts run` はモード分岐の前にこの検査を適用する (controlled 経路は
  別の場所で衛星を構築するため検査を通っていなかった)。([#335](https://github.com/sksat/orts/pull/335))
- 単位四元数でない `initial_quaternion` を、積分の前に正規化するようになった。
  したがって t=0 の出力に出るのも正規化後の値になる。config は元から非ゼロの
  四元数を受理していたが、生の値を積分すると大きな四元数がノルムが overflow する
  まで成長していた。([#335](https://github.com/sksat/orts/pull/335))
- `config` テーブルのない `[satellites.controller]` が、guest 自身の既定値で
  起動するようになった。省略時は文字列 `"null"` が guest に渡り、`init` が
  失敗していた。([#335](https://github.com/sksat/orts/pull/335))
- README のクイックスタート設定が、書かれたとおり動くようになった。
  `pd-rw-control` example plugin で RW を駆動する構成にした。従来は `sensors` と
  `[satellites.reaction_wheels]` を controller なしで宣言しており、
  それらを command するものが無かった。([#335](https://github.com/sksat/orts/pull/335))

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
- `arika_wasm::orbit_derived_batch` が、状態ベクトルの配列から Kepler 要素と
  軌道のスカラー量を返すようになった。ブラウザが `.rrd` を読むときも、CLI が CSV に
  書くのと同じ `KeplerianElements::from_state_vector` で計算される。軌道面を持たない
  状態 (`r = 0` や `r × v = 0`、後者は `v = 0` を含む) は 0 ではなく `NaN` を返す。
  0 は円赤道軌道の角度として実在する値なので、値なしの印には使えない。
  ([#376](https://github.com/sksat/orts/pull/376))
- 要素セットのパース ([#87](https://github.com/sksat/orts/pull/87))。共有の
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
  (`sgp4::Sgp4Propagator`): `Sgp4Elements` から構築し、epoch の
  `Constants` を再利用して TEME の `(Vec3<Teme>, Vec3<Teme>)` 状態 (km / km·s)
  へ伝播。`sgp4` crate を AFSPC compatibility mode (WGS72) でラップ。依存は
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
- `frame::MeanEquinoxOfDate` marker — 日付の平均赤道・平均春分点 (MOD)、`Eci`
  category。古典的な解析級数が基準にし、GMST が測られる equinox。
  `earth::mean_equinox` が `Gcrs` との間の IAU 1976 precession を持つ
  (`Rotation<MeanEquinoxOfDate, Gcrs>::iau1976_precession` と逆向き)。局所時角
  `GMST + λ − α` を組む利用者が、赤経を GMST と同じ equinox のフレームに置ける。
  ([#359](https://github.com/sksat/orts/pull/359))
- `EopTable::clamped()` / `EopTable::into_clamped()` → `ClampedEop`。範囲外の
  照会に最近端点の値を返す EOP provider。dUT1 は連続量 `UT1 − TAI` 経由で保持する
  ので、テーブル終端より後のうるう秒は UT1 に段差を作らず dUT1 を 1 s 動かす。
  ([#359](https://github.com/sksat/orts/pull/359))

#### Changed
- `Epoch::from_iso8601` が ordinal / day-of-year 形式
  (`YYYY-DDDTHH:MM:SS`、CCSDS OMM で使用) も受理し、末尾の `Z` が任意になった。
  厳密な緩和で、従来受理した入力は引き続きパース可能。([#87](https://github.com/sksat/orts/pull/87))
- **BREAKING**: `EopTable` は EOP capability trait
  (`Ut1Offset` / `PolarMotion` / `NutationCorrections` / `LengthOfDay`) を実装しない。
  これらの trait は infallible で、有限の MJD 区間しか覆わないテーブルは範囲外で
  正しい infallible な答を持たない (従来は `.expect()` で、通常の範囲外 epoch が
  `Epoch::to_ut1` や IAU 2006 full chain の内側からプロセスを abort させていた)。
  `table.clamped()` (借用) / `table.into_clamped()` (所有) で範囲外 policy を
  名指しするか、`*_checked` accessor で `EopLookupError::OutOfRange` を受ける。
  ([#359](https://github.com/sksat/orts/pull/359))
- `KeplerianElements::from_state_vector` の退化幾何の規約を型の doc に明記した
  (円軌道 / 赤道軌道 / 円赤道軌道で `raan` / `argument_of_periapsis` /
  `true_anomaly` が何を保持するかの表)。面内角は軌道法線まわりに測る `atan2`
  ベースのヘルパ 1 本で計算し、従来の `acos` + 象限判定が ν = 0 / i = 0 付近で
  落としていた mantissa の半分を回復した。非退化な軌道の値は変わらない。([#359](https://github.com/sksat/orts/pull/359))

#### Fixed
- `KeplerianElements::from_state_vector` が離心率のある赤道軌道の近地点方向を
  失っていた。RAAN と argument of periapsis の両方を 0 にしつつ真近点角を離心率
  ベクトルから測っていたため、赤道面内の近地点経度がどの要素にも保存されなかった。
  a = 10,000 km, e = 0.2, i = 0, ϖ = π/2 が 90° 回転して戻り、往復の位置誤差が
  11,313.7 km。赤道軌道では true longitude of periapsis ϖ = Ω + ω を保存する
  (逆行では符号反転。`to_state_vector` の i = π と整合)。([#359](https://github.com/sksat/orts/pull/359))
- Meeus の太陽・月暦が mean equinox of date のベクトルを `Vec3<Gcrs>` として
  返していた。平均黄経の係数が tropical rate で、黄道 → 赤道の回転も of-date の
  平均黄道傾斜角を使うため、J2000 からの累積歳差がそのまま乗っていた: 2024 年で
  0.335°、~1.4°/century で増加し、月ベクトルで約 2,250 km の横方向誤差 — 級数
  自身の ~1′ 精度より 1 桁大きい。IAU 1976 precession で J2000 に戻す (nutation
  ≤ 17″ と J2000→GCRS frame bias ~20 mas は入れない)。太陽・月に依存する結果
  (SRP、第三体重力、日陰、sun sensor) はこの角度ぶん動く。従来「Meeus model の
  精度 0.35°」と記録されていた量は、この回転ぶんだった。([#359](https://github.com/sksat/orts/pull/359))
- `sun::sun_direction_from_body` の惑星分岐が、Standish の惑星要素 (J2000 mean
  ecliptic 基準) を **of-date の** 黄道傾斜角で赤道座標に回していた (2024 年で
  11″、2075 年で 35″ の frame error)。固定の J2000 傾斜角を使う。([#359](https://github.com/sksat/orts/pull/359))
- `EopTable::dut1_checked` がうるう秒を跨いで dUT1 を直接補間し、1 s の跳びの
  半分を前日に塗り広げていた。2017-01-01 を挟む IERS の 2 行 (−0.5928 s /
  +0.4068 s) で中点が ≈ −0.593 s ではなく −0.093 s になり、UT1 で 0.5 s
  (ERA で 3.7e-5 rad、赤道で約 230 m) の誤差。連続量
  `UT1 − TAI = dUT1 − (TAI − UTC)` を補間し、照会時点の `TAI − UTC` を足し戻す。
  ([#359](https://github.com/sksat/orts/pull/359))

### `utsuroi` (Rust, crates.io)

#### Added
- `IntegrationError` が `core::error::Error` を実装 (手書き、`thiserror` 不使用、
  `no_std` でも動作)。`?` 連鎖や `Box<dyn Error>` に乗るようになった。([#147](https://github.com/sksat/orts/pull/147))

#### Fixed
- `Integrator::try_integrate` が、結果が非有限になった最初のステップで
  `IntegrationError::NonFiniteState` を返して停止するようになった
  (`integrate_with_events` が既に行っていた検査)。従来は `NaN` 状態のまま
  span 全体を回して `Ok` を返していたため、`orts serve` の制御ループが
  その状態からセンサを読み、plugin controller に渡し、成功を報告していた。([#335](https://github.com/sksat/orts/pull/335))

### `tobari` (Rust, crates.io)

#### Added
- NRLMSISE-00 の 72.5 km 未満: 中間圏・成層圏・対流圏の温度 spline と完全混合への
  線形遷移を実装し、地表から ~1000 km までをカバーするようになった。従来はそれ未満の
  すべての高度に 72.5 km の profile を黙って返していた (海面で 1.9e4 倍薄い)。
  新規 fixture 792 点での pymsis に対する最大密度誤差 0.0003%。72.5 km 未満は
  参照実装と同じく完全混合種 (N₂, O₂, Ar, He) と総質量密度のみを返し、
  O・H・N・anomalous O は 0。成層圏 spline と対流圏 spline が接する 32.5 km では、
  下側 spline の寄与が消えても node を埋めるので、NaN でなく値が返る。
  ([#361](https://github.com/sksat/orts/pull/361))
- `nrlmsise00::ApMode` と `Nrlmsise00::with_ap_mode` で、モデルを駆動する地磁気入力を
  選べるようになった: `Daily` (参照の既定、`ap_daily`) と `ThreeHourly`
  (`ap_array`、sub-daily な storm を解像)。3 時間の定式化は従来到達不能で、
  `ap_array` は死んだ入力だった。([#361](https://github.com/sksat/orts/pull/361))

#### Fixed
- NRLMSISE-00 の fixture が 72.5 km の分岐点を覆うようになった。下層は 72.4 km で
  止まり熱圏 grid は 100 km から始まるため、2 つの定式化が接する高度と、その上の
  72.5〜100 km (温度 spline を 72.5 km での `gts7` 評価に接続する帯) には突き合わせる
  参照値が無かった。72.5〜99.9 km の pymsis 点 432 件について、密度・温度・報告される
  7 種すべてを検証する。最大密度誤差 0.0020%、最大温度誤差 0.0005%。分岐点を 1 段
  ずらすと species の検証が落ちる。([#361](https://github.com/sksat/orts/pull/361))
- NRLMSISE-00 の季節項が 2π/365.25 の角速度を使っていた。係数セット自身の fitted 値は
  DR = 1.72142e-2 = 2π/365 で、doy 365 で季節位相が 0.25 日ずれる。1152 点の熱圏 grid で
  pymsis に対する平均密度誤差が 0.0886% → 0.0155%、最大温度誤差が 0.0354% → 0.0005%。
  ([#361](https://github.com/sksat/orts/pull/361))
- `HarrisPriester` が public な `u32` の指数を `powi(n as i32)` に渡していた。
  `n >= 2^31` で負に wrap し、`n = 2^31` は anti-bulge で `+Inf`、
  `n = u32::MAX` は `rho_min` の約 1280 倍を返していた。あらゆる `u32` に対して
  密度が `[rho_min, rho_max]` に収まるようになった。
  ([#360](https://github.com/sksat/orts/pull/360))
- `CssiSpaceWeather` が 3 時間 Ap 履歴と前日 F10.7 を record 配列の位置で解決していた。
  欠測日があると両方が時間方向にずれ、1 日の gap をまたぐと「3 時間前」が 27 時間前を
  指していた。どちらも暦日で引くようにし、データセットが覆わない日は問い合わせた日の
  日平均を fallback にした。このリポジトリの CSSI test fixture 自体に該当する gap が
  3 箇所ある。([#360](https://github.com/sksat/orts/pull/360))

#### Changed
- `CssiData::truncate_after` が `Result<CssiData, CssiParseError>` を返すようになった。
  データセット全体より前で切ると空の `CssiData` ができ、`CssiSpaceWeather::new` が
  それを受理して以降のすべてのクエリが panic していた。`CssiData` の構築はすべて
  `from_records` を通るようになり、空を拒否し重複日を畳む。
  ([#360](https://github.com/sksat/orts/pull/360))
- CSSI 宇宙天気ダウンロードの feature `fetch` を `fetch-cssi` にリネーム。
  `fetch-<source>` 規約(`fetch-igrf`、arika の `fetch-horizons`)に揃えた。
  `fetch` は全 `fetch-*` 源を束ねる傘 feature として存続するため、
  `features = ["fetch"]` は引き続きビルド可能(加えて `fetch-igrf` も有効化)。([#150](https://github.com/sksat/orts/pull/150))

### `tobari-wasm` (Rust)

#### Fixed
- `atmosphere_latlon_map`、`atmosphere_latlon_map_sw`、`atmosphere_volume`、
  `atmosphere_volume_sw`、`magnetic_field_latlon_map`、`magnetic_field_volume` が
  grid の次元を `u32` で乗算してから widen していた。大きな grid では確保が wrap する
  一方でループは全点を回る。0 次元では volume header の 2 要素だけを返し、
  doc が約束する `n_alt × n_lat × n_lon + 2` を満たさなかった。各 grid entry point は
  総数を `usize` で計算し、0 次元を拒否し、`MAX_GRID_POINTS` (2^24) を超えたら確保を
  試みずに JS 例外を投げるようになった。([#360](https://github.com/sksat/orts/pull/360))
- `magnetic_field_lines` が 0 や非有限の `step_km` を受理しており、その場合 trace が
  `max_steps` を使い切るまで走っていた (seed あたり最大 2^32 反復)。`n_seeds x max_steps`
  にも上限が無かった。そうした `step_km` を拒否し、総数を `MAX_FIELD_LINE_POINTS` で
  抑え、backward leg は各点を先頭に挿入する (leg 長に対して 2 乗) のでなく反転して
  一度に append するようにした。([#360](https://github.com/sksat/orts/pull/360))

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

#### Fixed
- source の中心天体定数を、source が名乗った天体から解決するか、解決できなければ
  source を拒否するようになった。従来は `mu` と半径がそれぞれ独立に地球へ fallback して
  いたので、Mars を名乗ってどちらも持たない recording が「Mars の `mu` + 地球の半径」
  として読まれていた。存在しない天体で、高度 (`r - radius`) が約 3000 km ずれるのに
  チャート上にその手掛かりが無い。既定値は viewer が定数を持つ天体に属するものとし、
  `DEFAULT_BODY_CATALOG` (arika が伝播する 10 天体) に置いた。そこに無い天体でも、
  source が両方の定数を持っていれば通す。何も発明していないため。source が持っている値が
  値になりえない場合 (`mu` が 0 以下、半径が 0 以下、いずれも非有限) は、既定値で
  差し替えるのでなくエラーにする。天体名を持たない source は従来どおり地球として読む
  (この field より古い recording がそれに当たる)。解決は metadata が届いた時点で 1 度だけ
  行い、その値を派生値と、チャートを組み立てる `SimInfo` の両方で使う。ファイル経路は
  読み込みの最後に field ごとに解決していたので、その時点で既に point がチャートへ
  届いていた。live の WebSocket source も同じ経路で解決するので、ファイルなら拒否される
  天体で読まれることはない。独自の天体で simulation する consumer は、その定数を adapter
  に渡す ([#383](https://github.com/sksat/orts/pull/383))
- `.rrd` ファイルを開いたとき、recording が持たない軌道量を導出するようになった。
  decoder が復元するのは位置と速度で、Kepler 要素とチャートが描く高度・比エネルギー・
  角運動量は 0 のハードコードで届いていた。400 km 軌道の recording が半長軸 0 km、
  高度 0 km として描かれていた。チャートの行はこれらを状態ベクトルから再計算せず
  point から直読するので、DuckDB の derived 列では埋まらなかった。2 つめの recording を
  開くときは導出の基準にする中心天体をリセットするので、decoder のメッセージがどの順で
  届いても、効くのは新しい recording の定数になる。負の天体半径は、無い場合と同じく地球に fallback する。
  高度は `r - bodyRadius` なので、地表からでなく軌道からの高さとして描かれてしまう。
  ([#376](https://github.com/sksat/orts/pull/376))
- multi-satellite チャートが schema 変更に追従するようになった。hook は起動時にしか
  chart Worker へ schema を伝えていなかったので、中心天体を変えた後は新しい schema で
  行を作る一方 Worker は古い schema で読み、derived SQL に前の天体半径と `mu` が
  残っていた。([#341](https://github.com/sksat/orts/pull/341))
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
- Worker message `update-schema` / `multi-update-schema` と
  `ChartDataWorkerClient.updateSchema()` / `MultiChartDataWorkerClient.updateSchema()`
  を追加。init 後の schema 変更が Worker に届く。([#341](https://github.com/sksat/orts/pull/341))
- store に `withTransaction`、`insertRows`、`replaceRows`、`replacePoints` を追加。
  全部入るか何も入らないかのどちらかになる書き込み。([#341](https://github.com/sksat/orts/pull/341))
- `initDuckDB` が DuckDB-wasm の worker / wasm を、jsDelivr CDN でなく
  呼び出し側が注入する self-host bundle URL からロード可能に。新しい
  `DuckDBInitOptions` (`bundles?`、`fallbackToJsDelivr?`) と `DuckDBBundleUrls`
  型、純粋関数 `resolveBundleSource(options?)` を追加。uneri は bundler 中立の
  まま、アプリ側が URL を解決して渡す。([#171](https://github.com/sksat/orts/pull/171))
- 堅牢な init: `initDuckDB` は linear backoff でリトライし、死んだ worker は
  `error` listener で即 fail (ハングしない)、terminal failure 後はキャッシュ
  された reject promise を破棄して次回呼び出しでリトライする。([#76](https://github.com/sksat/orts/pull/76), [#70](https://github.com/sksat/orts/issues/70))

#### Changed
- `insertPoints` が atomic になった。1,000 行ごとの batch の途中で失敗した場合、
  成功済みの batch も残らない。自前でトランザクションを開くため、同一 connection
  で並行呼び出しすると後続がエラーになる (DuckDB にネストしたトランザクションが
  無い)。呼び出しは逐次にする。([#341](https://github.com/sksat/orts/pull/341))
- 引数なしの `initDuckDB()` の既定動作は不変 — 引き続き jsDelivr CDN から
  bundle を取得するため既存 consumer はそのまま動く。self-host は
  `options.bundles` で opt-in。([#171](https://github.com/sksat/orts/pull/171))

#### Fixed
- chart data Worker が init 時の schema で derived 列を計算し続けるため、後から
  中心天体が変わっても反映されなかった (地球→月で `altitude` が 4,640.737 km
  ずれる)。drain 処理が、変更後の schema をその schema で作った行より先に送る。([#341](https://github.com/sksat/orts/pull/341))
- rebuild と insert が「DELETE + INSERT を N 回」で、途中失敗すると空または
  中途半端なテーブルが残り、再送でコミット済みの行が重複していた。1
  トランザクションにまとめて単位ごと再試行し (上限は単機版と同じ 3 回)、
  上限に達した rebuild はテーブルを空にして error を post する (古い dataset を
  残すと後続の行と混ざる)。([#341](https://github.com/sksat/orts/pull/341))
- `onmessage` が `async` のため rebuild と tick が互いに割り込み、rebuild 完了時の
  queue クリアで実行中に届いた行を捨てていた。全 command を 1 本の直列キューで
  処理し、新しい rebuild は古いものを supersede し、schema・表示窓・dataset の
  変化より前に始まったクエリの結果はキャッシュしない。([#341](https://github.com/sksat/orts/pull/341))
- 0 行の rebuild 後も前のチャートが残り続けた。空のデータを 1 回 broadcast する。([#341](https://github.com/sksat/orts/pull/341))
- `IngestBuffer.markRebuild` が、置換データの方が早く終わる場合に `latestT` を
  下げず、表示窓が実データより先に張られていた。([#341](https://github.com/sksat/orts/pull/341))
- multi-satellite Worker が per-satellite 状態の snapshot を反復していたため、
  ある衛星の INSERT 待ち中に別の衛星へ届いた行が消えていた。表示窓の基準も、
  行を持たない衛星を含んでいた。([#341](https://github.com/sksat/orts/pull/341))
- init 時の worker 404 / "invalid URL": bundle URL を `initDuckDB` 内で worker
  origin に対して絶対化する。DuckDB が worker を `blob:` URL から生成するため、
  root-relative パスでは解決できないことへの対処。([#171](https://github.com/sksat/orts/pull/171))

### `rrd-wasm` (Rust, crates.io)

#### Fixed
- scalar 列を、列内の値の位置ではなく recording 自身の時刻 index で結合するように
  なった。scalar の各成分は独立した entity path (`<base>/x`, `<base>/y`, …) に載り、
  両者が一致するのは全列が全 step で値を持つ間だけで、一部の step にしか出てこない
  成分は後の値が前の行にずれ込んでいた。`y` が t=10 にだけ logging されている場合、
  t=0 の行にその値が載り、t=10 の行は `y = 0.0` になる。行を出すのは position 3 成分
  (recording が velocity 列を持つ場合は velocity 3 成分も) がその時刻に揃ったときで、
  揃わない行はゼロ埋めせず落とす。`orts run --format rrd` は 1 回の run のすべての
  step で同じ component を logging するので、その出力のデコード結果は以前と同じ。疎な
  列がこの結合に入るのは、外部で書かれた `.rrd`。`orts serve` の history segment は
  attitude が state ごとに optional なので疎になりうるが、`save_as_rrd` が component の
  row `i` を entity の timeline の row `i` に書くため、遅れて始まる attitude はファイルの
  時点で既に誤った時刻にある。こちらは writer の修正が先に必要。独自の名前の timeline で index された recording も、
  列の位置に落ちるのでなくその timeline で結合する (`sim_time` と `step` 以外はすべて
  列の位置になっていた)。その名前は recording 全体で 1 つで、`sim_time` と `step` と
  並んで行を識別する。ある index が一致していても別の index の値が違えば同じ行には
  載らない。([#366](https://github.com/sksat/orts/pull/366))

### Docs

#### Added
- ドキュメントサイトに `llms.txt` / `llms-full.txt` / `llms-small.txt` を生成
  (`starlight-llms-txt`)。coding agent や LLM ツールが docs を取り込めるようにした
  — 例: <https://sksat.github.io/orts/llms.txt> を agent に渡す。`llms-full.txt`
  は全文、`llms-small.txt` は自動生成 API リファレンスを除いた要約版。([#225](https://github.com/sksat/orts/pull/225))

### Dependencies

- Rust toolchain → 1.96.0。
- Rust: `wasmtime` / `wasmtime-wasi` 44 (security)、`rerun` 0.33、
  `tokio-tungstenite` 0.29、`nalgebra` 0.35、`tokio` 1.52、`axum` 0.8.9。
- `notalawyer` 0.3 — 埋め込む third-party ライセンス NOTICE を、`cargo about`
  バイナリでなく cargo-about **ライブラリ**(`orts-cli` の build-dependency)で
  生成するようにした。CI でのバイナリ install と cross ビルドイメージへの
  埋め込みが不要になった。
- npm: `vite` 8、`@vitejs/plugin-react` 6、React monorepo、`ws` 8.21
  (security)、`mermaid` 11.15 (security)。

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

