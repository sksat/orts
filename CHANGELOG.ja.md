# Changelog (日本語)

[Keep a Changelog](https://keepachangelog.com/ja/1.1.0/) に緩く準拠。
[Semantic Versioning](https://semver.org/) で versioning。

orts は マルチパッケージ workspace (crates.io Rust crate + npm package)。
全パッケージを同一バージョンでリリースし、セクションはパッケージ別に分割。

## [Unreleased]

### `orts` (Rust, crates.io)

#### Added
- `PropellantPool` を追加した。宇宙機が積む推進剤をプール 1 つ・床 1 つで表し、残量は state が
  床より上に持っている質量である。床 0 は拒否する (`F/m` の特異点に床を置くことになる。境界の探索は
  交差前のモードで区間を刻み直すので、床を越えて試行するのが設計である)。
  `SpacecraftDynamics::with_propellant` がプールを、`with_propulsion` がそれを消費する model を
  登録する。推進系の model は他の model と同じに評価され、プールが空になったら一切評価されない。
  どの model が推進剤を使うかは呼び出し側が言う: 質量流量の符号や名前から推測すると、drag や
  reaction wheel まで止めてしまう。
- `SpacecraftDynamics::load_breakdown` を追加。加速度の magnitude と body torque を、
  全モデル 1 回の評価から `LoadBreakdown` (`orts::spacecraft` から re-export) で返す。`ExternalLoads` が両方を持っているので、両方を欲しい
  呼び出し側 (1 サンプルを報告する telemetry) が全モデルを 2 回評価する理由はない。
  パネル 22 枚の機体では 1 回 50 µs の影の幾何を二重に払っていた。片方だけを返す
  accessor は従来どおりで、それぞれ `model_breakdown` から自分の分だけを作る。
  `load_breakdown` を経由すると呼び出し側が捨てる半分を確保することになり、トルクだけを
  問う呼び出し (`orts run` の各出力サンプル) には重力場の評価も要らない。
  ([#470](https://github.com/sksat/orts/pull/470))
- `orts::eclipse` を追加。太陽を遮る天体を一覧で持ち、中心天体以外も遮蔽体になれるようにした。
  `OccultingBody` が天体の位置・半径・遮蔽の幾何を持ち、`default_occulters` が中心天体ごとの
  標準の一覧を返し、`illumination` が力モデルやセンサが使う 1 つの照射率にまとめる。これで
  月周回の衛星が地球の影に入る。2026 年で実測すると、100 km の月周回軌道は年 4.0〜7.7 時間を
  地球の影の中で過ごし、連続では最長 257 分、日付は月食の日である。Orekit 13.1.7 はその
  幾何で lighting ratio 0.000 を返し、修正前のこのモデルは 1.000 を返していた。
  互いの視円板が離れている天体は太陽の別の部分を隠すので割合を足す。一方の視円板が他方の
  内側にあれば手前の天体が奥ごと隠しているので大きい割合を採る。ここまでは厳密である。
  どちらでもない重なりでは隠す部分を共有する。厳密な答えは 2 円の和集合を第 3 の円で切った
  面積で、これは計算しておらず `a + b - ab` を代わりに使う。この値は大きい方と和の間に入り、
  部分食 2 つから皆既を作らない。この場合は月の一覧でも起きる。月食の間に衛星が月の
  terminator を通るときである。この幾何の規則は 2 天体まで、つまり library が作る一覧の
  範囲である。3 天体以上では、視円板が触れ合う天体を群にまとめ、群の間は足し (厳密)、
  群の中は対称な `1 - Π(1 - aᵢ)` で合わせる。並び順に依存せず、1 天体が皆既でなければ
  皆既にならない。遮蔽の幾何はモデルではなく天体が持つ。円柱近似は
  周回している天体では食の長さの 0.5% の違いだが、月周回から見た地球のように遠い天体では
  1.86 倍になるので、遠い遮蔽体は中心天体が何であれ円錐で扱う。
  ([#469](https://github.com/sksat/orts/pull/469))
- `SpacecraftDynamics::torque_breakdown` を追加。モデルごとの外乱トルクを機体座標系 [N·m]
  で返す。既存の `acceleration_breakdown` が magnitude を返すのに対しこちらはベクトルを
  返す。magnitude は符号も軸も運ばないので、機体を逆向きに回す外乱と正しい向きに回す外乱が
  同じ値になる。その向きを直したのが平板 SRP の反射項 [#377](https://github.com/sksat/orts/pull/377) と大気抵抗の面の
  選び方 [#437](https://github.com/sksat/orts/pull/437) である。重力場は一覧に
  出ない。重心に働くので重心まわりのトルクを作らないためで、gravity gradient のトルクは
  モデルとして自分の名前で出る。([#466](https://github.com/sksat/orts/pull/466))
- `Recording::log_temporal_scalars` を追加。実行時に決まる名前で component を記録し、
  field 名も一緒に registry へ登録する。`Recording::log_temporal` はこれに委譲する。
  モデルごとのテレメトリ列は型から名前を取れず、`.rrd` の schema は与えた名前をそのまま
  運ぶ。名前を作るのは `ModelTorqueBody3D` と `torque_columns` で、component 名は
  `orts.ModelTorqueBody3D:panel_srp`、field 名は `panel_srp.torque_body_x_Nm` になる。
  モデル名は `[A-Za-z0-9_]` へ可逆に符号化するので、1 文字の置換で一致する 2 つのモデルも
  別の列になる。同名の繰り返しは `.2`, `.3` で分ける。往復の実測で `[`, `]`, `*`, `#` は
  entity path から戻らない。([#466](https://github.com/sksat/orts/pull/466))
- `Model::eval_in_segment` と `ThrustProfile::throttle_in_segment` を追加。引数の
  `EvalSegment` は segment を integration time と開始時刻の絶対時刻の両方で運ぶ (`epoch_0` を
  持つのは system で model ではない)。既定は `eval` / `throttle` への転送で、`ScheduledBurn` は
  segment 開始時刻の値を返す。これで区間 `[a, b)` は `b` に乗る stage でも on のままになる。
  model を評価する 5 つの system と group の composite 2 つが segment を配下に転送する。
  telemetry は `eval` のままなので、`model_breakdown` は区間の終端で区間外を返す。([#453](https://github.com/sksat/orts/pull/453))
- `StateEffector::derivatives_in_segment` と
  `InterSatelliteForce::acceleration_pair_in_segment` を追加 (既定は stage 時刻版への転送)。
  どちらも境界を報告できるので、segment のあいだ保持すべき schedule を持ちうる。衛星間力の
  引数が `SegmentContext` なのは、`PairContext` が epoch を運ばないため。([#453](https://github.com/sksat/orts/pull/453))
- `perturbations::SphericalHarmonicGravity<F: EarthFixedTransform>` —
  `tobari::gravity::SphericalHarmonicField` (EGM96 / EGM2008 / EIGEN 系の ICGEM
  file から読んだ `SphericalHarmonicCoefficients` の degree × order 窓) による
  完全球面調和重力。`F` の地球固定 chain で回す: `SimpleEci` は ERA
  のみ、`Gcrs` は極運動込みの IAU 2006 CIO chain。非中心項のみ (`PointMass` と
  併用し、`ZonalGravity` とは同時に登録しない)。絶対 epoch が無い場合は J2000 への
  fallback ではなく panic。70×70 で 24 h の LEO 伝播が Orekit (ITRF body frame,
  実 EOP) と 0.93 m で一致。([#411](https://github.com/sksat/orts/issues/411))
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

#### Added
- `setup::build_orbital_system_in_frame::<F>` — 慣性系を明示する
  `build_orbital_system`。frame-aware なモデル (drag、球面調和場) がそれぞれ
  provider を持てるよう EOP storage を factory で受ける。`HasPosition` を任意の
  frame の `OrbitalState<F>` に実装し、`record::SimMetadata` に frame 名を追加。
  ([#411](https://github.com/sksat/orts/issues/411))
- **BREAKING**: `setup::build_orbital_system` / `build_orbital_system_in_frame` /
  `build_spacecraft_dynamics` が、別々の `mu` と `gravity_field` 引数の代わりに
  `setup::CentralGravity` (`Zonal { mu }` または `Harmonic(field)`) を受ける。
  中心項の GM の出所が一つ (`CentralGravity::mu()`、`Harmonic` では場自身の GM) に
  なるので場の GM と食い違えず、zonal / harmonic の排他は builder 内のチェックではなく
  variant で表される。([#411](https://github.com/sksat/orts/issues/411))

#### Changed
- **Breaking:** reaction wheel が、トルクが指令にどう追従するかを `TorqueResponse` で表すように
  なった (`Rw::motor_time_constant: Option<f64>` を置き換える)。variant がモデルを、field が量を
  名指しする: `with_motor_lag(0.05)` では 50 ms が時定数か無駄時間かを言っておらず、
  `motor time constant` ではモータの電気的時定数・機械的時定数と紛れるので、
  `TorqueResponse::first_order_lag(0.05)` とする。`None` にも名前が付いた
  (`TorqueResponse::Instant`、既定値)。`with_motor_lag` は deprecated な別名として残す。
  `RwAssemblyCore::has_motor_lag` は `carries_torques` になった。これが決めているのは
  「状態が角運動量の隣にトルクを持つか」で、wheel の性質ではない: 即応の wheel も、トルクを持つ
  assembly では枠を 1 つ持ち、その値は telemetry 用に指令へ追従する。variant は直接書けるので、
  `RwAssembly::new` が渡された wheel の応答を検査する。
- **Breaking:** `BoundaryKind` に 4 つめの variant `TurningPoint { index }` が増え、
  `BoundaryKind::mode_after` の戻り値が `ConstraintMode` から `Option<ConstraintMode>` になった。
  `None` は「状態もモードも動かさない境界」を表す。variant を `match` している側と、境界が移る先の
  モードを使っている側は、どちらも対応が必要になる。`HasBoundaries::settle_boundary` を自分で
  実装している system は、`None` を見た時点で戻る必要がある (step を割るだけの境界は何も settle
  しない)。
- **BREAKING**: reaction wheel は角運動量を保持できなければならない。`Rw::new` と
  `Rw::with_max_speed` は、ホイールが最終的に持つ上限 `max_momentum.min(inertia * max_speed)` が
  正でなければ panic する。`RwAssemblyCore::new` も、作られた後に容量を失ったホイールで panic する:
  `Rw` のフィールドは pub で `momentum_limit` がそれを読むので、`rw.max_speed = 0.0` だけで
  容量 0 になる。上限 0 では境界を位置付ける対象がない — 角運動量の余裕が 0 から始まるので探索は
  「すでに越えた」と読み、ホイールは最初のステップから保持されて何も吸収できず、境界付近の丸めの
  ために置いた許容幅 (1e-12 N·m·s) がホイールの可動範囲そのものになる。検査を最終的な上限に対して
  行うのは、`Rw::new` が速度上限を除算で導いており、慣性が大きいとアンダーフローするためである
  (`f64::MIN_POSITIVE / f64::MAX` は 0 rad/s)。`PropellantPool::new` が dry mass に課しているのと
  同じ要求である ([#529](https://github.com/sksat/orts/issues/529))
- **BREAKING**: `orts::boundary::walk_to_target` の戻り値が utsuroi の
  `IntegrationError` から `BoundaryWalkError` になった。系の拘束が受け付けない state は
  積分の失敗ではないので分けている: `BoundaryWalkError::StartRejected { t, error }` が
  `StartStateError` を持ち、「何が拒否し、何を読んだか」に加えて、群の場合はどの衛星かを
  持つ (群は 1 つの合成 state として刻むため、直せばよい衛星ではなく最初に刻んだ衛星を
  記録してしまう)。solver の失敗は `BoundaryWalkError::Integration` のままである。エラーを match する呼び出し側は腕を `Integration` で包む。整形するだけの
  呼び出し側はそのままで、group が終了した衛星に記録する理由の文面も変わらない。答えるのは
  `HasBoundaries::validate_boundary_walk_start` と `StateEffector::validate_state` で、
  どちらも既定は `Ok(())` である。前者は walk の開始時刻を取る: 系が拘束の読む値を
  与えている場合があるためで、`AugmentedAttitudeSystem` は質量を時刻の関数から取るので、
  同じ state が或る時刻では dry mass より上、別の時刻では下になる。dry mass を下回る質量、質量と食い違うプールのモード、
  上限を超えたホイール、上限から離れているのに保持と言っているモード、そして `aux` /
  `modes` の長さや `aux_bounds` の値が登録した effector の申告と合わない state は、境界を処理する経路の
  入口で拒否される。独立群、結合群 (各衛星に聞く)、`AugmentedAttitudeSystem`、CLI の制御付きループ、
  `walk_to_target` の直接呼び出しである。`Integrator::integrate` で直接刻む経路は境界を処理しないので
  検査も走らない (モードが動かないのと同じ制限である)。`Scheduler` は群を組む前に各衛星へ聞き、
  開始できない機体を落としてから残りをその区間ぶん飛ばす — 結合群の walk は 1 機の拒否で
  1 ステップも積分しないので、そのままでは同じ component の衛星が、scheduler の時計が
  既に通過した時刻に取り残される。`CoupledGroupParts::is_event_termination` は
  `stop: Option<ComponentStop>` になった。walk が event で終わったのか、開始 state の拒否か、
  積分エラーかを区別する: scheduler が component に対して何をするかは 3 通りで、bool 1 つでは
  表せない。以前はどれも無言だった: dry mass 100 kg に対して手組みした 99.5 kg の
  state は、最初のステップを終えると 100 kg になっていた — 入力に無かった 0.5 kg の推進剤が
  増えていた。state が既に越えている境界を処理するのは、伝播ループの仕事だからである
  ([#523](https://github.com/sksat/orts/issues/523))
- **BREAKING**: thruster も assembly も dry mass を持たなくなった。
  `ThrusterSpec::dry_mass` / `Thruster::with_dry_mass` / `ThrusterSpec::with_dry_mass` と
  `ThrusterAssemblyCore::new` の第 2 引数を削除し、床は宇宙機のもの (`PropellantPool`、dynamics に
  登録する) にした。推進器がそれぞれ床を持つと、いつ宇宙機が空になるかで食い違える。さらに、
  それぞれが RHS の中で行っていた比較 (`state.mass <= dry_mass`) は境界の探索と両立しない:
  刻み直した区間のステージ間で切り替わり、交差の時刻を誤って報告する。`orts.toml` も同じ要求で、
  `[satellites.thruster] dry_mass` は必須で正の値である (以前は既定値 0)。
- **BREAKING**: `SpacecraftDynamics` の 4 つの load breakdown (`model_breakdown` /
  `torque_breakdown` / `acceleration_breakdown` / `load_breakdown`) が
  `&SpacecraftState<F>` ではなく `&AugmentedState<SpacecraftState<F>>` を取る。噴射するかどうかは
  プールのモード、つまり右辺と同じ値で決まり、モードは augmented state にある。質量を読む形では、
  境界を処理しない経路で記録と軌道が食い違った。`sat.state` を持っている呼び出し側は、`.plant` に
  入らずそのまま渡す。
- **BREAKING**: `BoundaryExchange` に `mass: Option<f64>` (境界が決める質量) が増えた。角運動量
  だけ返す effector は `BoundaryExchange { angular_momentum_body, ..Default::default() }` と
  書けばそのままコンパイルでき、この書き方なら後でフィールドが増えても壊れない。
- **BREAKING**: 状態が限界に達する時刻をどこまで詰めて求めるかを設定にした (これまでは
  各経路が既定の 1 ms を埋め込んでいた)。`IndependentGroup::with_root_search` と
  `CoupledGroup::with_root_search` が `RootSearch` を取り、`propagate_controlled` は
  積分器と event check の間で受け取る。`orts run` / `orts serve` は
  `--root-t-tolerance` / `[integrator] root_t_tolerance` を読み、正の有限値でなければ
  config の時点で拒否する (どの積分器も同じ探索を通るので、全てに適用する)。半分割の回数は許容から導く (許容を縛らない):
  CLI は `⌈log₂(widest / t_tolerance)⌉` 回に、`t = 0` 近傍の subnormal の範囲ぶんとして 64 回を
  足して要求する。`widest` は run の span である。`dt` は区間を縛らない (adaptive な 2 つでは
  最初のステップにすぎず、受理したステップを DP45 は最大 5 倍、DOP853 は 6 倍に繰り返し育てる)。
  縛るのは伝播そのもので、ステップは歩いている目標時刻で切られる。この許容は探索が
  交差を含む区間を詰める幅であり、返す時刻が「見つけた符号変化」からどれだけ離れうるかである:
  一定トルク $\tau$ で駆動されるホイールは上限を $\tau \cdot \varepsilon_t$ 超えたところで保持され、
  body はその分の交換を保つ。縛るのはこの絞り込みの分だけで、宇宙機が実際に境界へ達する時刻との
  差には state の積分誤差も乗る (符号変化は計算された軌道の上のものである)。
  下限は f64 の時刻分解能で、探索は半分割しても区間が変わらなくなった時点でも止まる
  ($t = 10^{15}$ では 0.25 秒の区間が 1 回で止まる)。半分にするごとに、狭めている区間の再計算が 1 回増える。上限が 5.3 秒 (5.25 から
  5.5 のステップの内側) に来るホイールでの実測では、1 マイクロ秒を指定すると 5.3 を報告し、
  0.2 秒を指定すると 1 回の半分割で許容に収まって 5.375 を報告する。
- **BREAKING**: `StateEffector` の評価コンテキストを `EffectorInput` 1 つにまとめ、
  effector が自分の state が到達しうる境界を申告するようにした。
  `derivatives(&self, t, state, aux, aux_rates, epoch)` は
  `derivatives(&self, input, aux_rates)` になる: `t` / `state` / `aux` / `epoch` は
  input のフィールドになり、rates を書き込む buffer は第 2 引数に残る。input は
  state の離散モード (`modes`) と積分中の segment も運ぶ。追加した trait method
  (`boundaries` / `boundary_value` / `settle_boundary` / `mode_dim`) は全て既定
  実装を持つので、境界のない effector は何も申告しない。併せて `AugmentedState` に
  `modes: Vec<ConstraintMode>` が増え (struct literal は名前を書く必要がある)、
  `AuxRegistry::register` は第 3 引数に effector の `mode_dim` を取り、group が
  伝播する `DynamicalSystem` は `HasBoundaries` も実装する必要がある (全 method が
  既定実装なので、境界のない系は `impl HasBoundaries for X {}` で足りる)。
  境界の値は導関数と同じ segment で読む。segment の間だけ値を保持する部分 (指令された燃焼など) は
  segment の始点について答えるので、評価している state が経験していない切替の向こう側の値を
  見なくなる。
  境界の値は残余である: 境界の手前で正、境界上でちょうど 0、越えると負。歩き始めの state は
  残余が負であれば (幅を持たせずに) 境界へ載せる — 負のままから先も負で、探索が報告できる符号変化は
  残っていない。effector の `boundary_tolerance` が抑えるのは、既に局在化した交差の 2 度目の報告で、
  それは探索側の guard である。到達は残余が尽きる 1 方向なので
  `EffectorBoundary` は交差の向きを持たず、上がって境界に入る量は符号を反転して書く。越えた側が
  1 つに決まることが、「歩き始めの state が既に境界を越えている」場合 (探索では見つけられない) を
  判定して刻む前に境界へ載せる根拠になる。
  これらの系に渡す state は、effector が登録した分のモードを必ず持つ。持たない state は最初の
  評価で panic する (長さの合わない aux vector と同じ扱い)。モードのない state はどの拘束が
  held かを言えないので、その effector が申告した境界は全て inactive と読まれ、group は walk を
  走らせたままホイールを上限の外へ運んでしまう。`initial_augmented_state` がこの vector を作る。
  ホイールの角運動量の上限を守るのは伝播になり、`orts::boundary::walk_to_target` を走らせる
  経路 — group、CLI の制御付き経路、あるいはこの関数を直接呼ぶ経路 — でだけ守られる。
  `Integrator::integrate` で直接刻む呼び出し側では全てのホイールが `Free` のままで、
  0.53 N·m·s のホイールを 0.1 N·m で 20 秒駆動すると 2.0 N·m·s に達する。
  飽和のような拘束を RHS の中の比較で表すことはできない: root の探索は同じ区間を
  幅を変えて再計算するので、ステップの途中で切り替わる比較は RK のステージ間でモードを
  混ぜ、到達時刻を `0.2r` 遅く報告する (実測、`DESIGN.md`)。モードは state が持ち、
  積分の中では誰も書き換えない。
- `IndependentGroup` と `CoupledGroup` が、どの solver も同じ 3 つの呼び出し
  (`stepper` / `from_checked_state` / `advance_to`) で進めるようになった (RK4 の枝だけが
  自前の step ループを持っていた)。それに伴い、そのループが adaptive stepper と違っていた
  2 点が変わる。固定刻みのエラー (state が非有限になる、あるいはその時刻のグリッドが運べない
  `dt`) は衛星の termination として記録されるようになった (従来は `propagate_to` の `Err` と
  して返っていた)。**`t = 1e15` の group を `0.1` 刻みで伝播すると、その衛星が
  `StepBelowSpacing` で終了し**、他の衛星は進み続ける。もう 1 点、**失敗した step の state を
  保存しなくなった** — RK4 はそれを代入し、adaptive stepper は最後に受理した state を
  保っていた — ので、`satellites()` と `into_parts()` が返す state は有限で、衛星自身の clock は
  最後に完了した segment の終端に留まる。termination が記録する内容は変わらない (失敗した
  step の着地時刻と、reason としてのエラー)。([#458](https://github.com/sksat/orts/pull/458))
- **BREAKING**: `SurfacePanel` に public な `outline: Option<PanelOutline>` が
  増えたので、struct literal はこの field を書く必要がある。`PanelSrp` と
  `PanelDrag` は、別のパネルに完全に覆われたパネルを飛ばすようになった (以前は
  全面照射として計算していた)。判定に参加するのは輪郭を持つパネルだけで、`at_com`
  で作ったパネルは従来と同じ力を出し、遮蔽もしないし遮蔽もされない。輪郭を持つ
  パネルは `SurfacePanel::rectangle` で作り、面積は半寸法から導出する。
  `SpacecraftShape::cube` の 6 面も輪郭を持つようになった (面自身の力は変わらないが、
  cube の隣に足したパネルが面の陰にいることを判定できる)。([#424](https://github.com/sksat/orts/pull/424))
- **BREAKING**: `setup::build_orbital_system` / `build_spacecraft_dynamics` に
  末尾引数 `gravity_field: Option<Arc<SphericalHarmonicField>>` を追加。`Some` で
  `SphericalHarmonicGravity` (Earth 専用、epoch 必須、`mu` は場の GM と一致すること
  を assert)、`None` で従来の `ZonalGravity`。([#411](https://github.com/sksat/orts/issues/411))
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
  明示して検査する `ComponentColumn::push_at` を通す。`push` は削除、`scalars_per_row` と
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
- **破壊的変更**: `SolarRadiationPressure::shadow_body_radius` と `shadow_model` を
  `occulters: Vec<OccultingBody>` に置き換えた。`PanelSrp` と `SunSensor` も同じ一覧を
  private に持つ。`without_shadow` / `with_shadow_body` / `with_shadow_model` は引き続き
  使える (順に、一覧を空にする / 原点の 1 天体で置き換える / 中心天体の幾何を設定する。
  遠い遮蔽体は必要な円錐のまま残る)。
  1 つ足すのは `with_occulter`。古いフィールドを名前で書いた struct literal は
  コンパイルできなくなる。([#469](https://github.com/sksat/orts/pull/469))
#### Fixed
- walk が、自分の system が拒否する状態を作った時点で報告するようになった (新しい
  `BoundaryWalkError::ProducedRejected`)。従来はその状態を返し、次の呼び出しが拒否していた。
  この状態に至る道は 3 つある。1 つは、持っている遅れに対して step が粗すぎる区間である: 既定 gain の
  速度指令、時定数 50 ms、step 幅 200 ms では、step の内側で評価される rate が両端にない符号を取り、
  上限を越える分だけを止める保持をそのまま通るので、角運動量が上限から離れる (上限 1.0 に対して
  0.9459 N·m·s で静止し、モードは `Upper` のまま。`validate_state` はこれを無効と呼ぶ)。この設定では
  実現トルクは step の終わりでどこも正で、step 幅 1 ms なら角運動量は 1.0 に保持される — 取りこぼしは
  なく、step 幅だけがこの状態を作る。2 つめは、探索が報告できない交差を含む区間である (1 step に 1 つの
  境界の値が 2 回符号を変えると、どちらも報告されない)。3 つめは、gate されていない contribution である:
  `with_model` で登録した thruster は pool の floor で止まらないので、その run は floor を下回った質量を
  返す代わりに floor を名指しして停止する。system が受け付ける状態には影響せず、
  1 次遅れより十分細かい step では従来どおり wheel は上限に保持される。この報告のために
  `ComponentStop` に `ProducedRefused` を足した (`StartRefused` は「何も積分しておらず、呼び出し側が
  渡した状態を拒否した」を約束するため)。coupled group はこれを積分エラーと同じ扱いにして component
  全体を止める (parts が直前に完了した segment を持つため)。
- 1 次遅れのある reaction wheel が、角運動量の増減が切り替わる時刻を新しい
  `BoundaryKind::TurningPoint` として申告するようになった。step 幅が 1 次遅れより広い場合でも、
  上限を短時間超える動きが拘束される。上限の直前で、まだ外向きに加速している wheel に逆向きの
  トルクを指令すると、角運動量は上限を超え、実際のトルクが 0 を通ったあとに戻る: 上限 1 に対して
  角運動量 0.999 N·m·s、トルク ±0.1 N·m、時定数 50 ms では、margin は 13.2 ms で 0 を下回り
  59.7 ms で 0 を上回る。どちらも 100 ms の step の内側で、その両端は 0.0010 と 0.0043 で同符号
  だったため、従来この step では拘束が一度も働かず、角運動量は wheel が保持できない
  1.000534 N·m·s (上限の 0.053% 超) まで達していた。超過の大きさは 1 次遅れの時定数で決まり、
  500 ms では同じ反転で 1.014 N·m·s (1.4% 超) になる。現在は 100 ms の step でも 13.7 ms で拘束され、
  トルクが転じる 34.8 ms で解除される。転じる時刻の前後では角運動量が単調なので、分割した区間では
  探索が符号差を読める。そのために `BoundaryKind::mode_after` の戻り値を
  `Option<ConstraintMode>` に変えた: 転換点は状態もモードも動かさず、walk の開始時に行う
  「すでに越えている境界の補正」の対象からも外す (トルクが負であること自体は正常な状態である)。
  1 次遅れのない wheel は申告しない。その `dh/dt` は指令値で、0 のまま置かれた指令は角運動量の
  転換ではない。
- `Synchronized` の速度更新を受ける衛星が、同じ同期区間でどれか 1 機の計算が終了判定で終わると、
  `propagate_to` の残りを積分されなかった。KDK 経路は閉じの速度更新を当てたあとループを抜けていたので、
  時計はその同期区間の終わりで止まり、呼び出しは飛ばした区間について何も言わずに `Ok` を返していた。
  実測では、一定の 0.02 m/s² で引き合う 2 機と、無関係な 3 機目が t = 101 s で範囲を出る条件で、
  1 回の `propagate_to(300)` が 120 s で戻り、速度更新を受ける衛星は 204 m だった (厳密解は 1050 m)。
  要求した 300 s のうち 180 s が誰にも積分されていない。伝播は要求された時刻まで続くようにした。
  あわせて、片方の端点が途中で終了したペアを閉じの速度更新から外した。従来はその端点の位置を
  止まった時刻のまま読み、反作用も片側にしか当てていなかった
  ([#530](https://github.com/sksat/orts/issues/530))。
- 推進剤の床をステップの内側で跨ぐ燃焼が、宇宙機が持っていない推進剤を使っていた。thruster は
  「質量が床にあるか」を渡された state で判定していたので、跨いだステップは全開で燃え続けた。
  [#446] の実測では、残量 0.04 kg・推力 196.133 N・1 秒刻みで、最終質量が床を 0.01 kg 下回り、
  ΔV は実在する推進剤ぶんの Tsiolkovsky 値 0.784375 m/s に対して 0.980273 m/s (24.98 % 過大)
  だった。枯渇は伝播が時刻を求める境界になった: `PropellantPool` が申告し、質量を床に載せ、
  そのモードがこの推進剤を使う全ての消費者を止める。
  質量のない state (探索は交差前のモードで区間を刻み直すので、そこに達するのは設計である) は
  `F/m` の領域の外なので、model と effector が返した加速度を系が落とし、残りはそのまま使う。
  これがないと導関数は `[inf, NaN, NaN]` になり、捨てる予定の state で walk が失敗する。残す方の
  質量流量は質量によらず、ステップの終端を床の向こうへ運ぶ項でもある: 潰すと床をまたぐステップの
  両端が床より上に残り、交差が次のステップまで報告されない。同じ条件で質量は床に乗り、ΔV は RK4 / DP45 /
  DOP853 のいずれでも Tsiolkovsky 値と 1e-6 m/s 以内で一致する。残るのは、交差を局在化する前に
  床より下で燃やした推進剤ぶんの力積 (`ṁ · t_tolerance`) である。1 ナノ秒の局在化なら 1e-10 kg で、
  2e-9 m/s に相当する。
- 角運動量の上限に達した reaction wheel が、宇宙機の全角運動量を減らしていた。
  0.53 N·m·s のホイールを 0.1 N·m で駆動する 20 秒の伝播で、body frame の合計
  `I·ω + Σ aᵢ hᵢ` は `dt = 0.25` の RK4 で 7.5e-3 N·m·s、`atol = rtol = 1e-3` の
  DP45 で 8.4e-2 N·m·s — ホイールの上限の 16% — 失われていた。ホイールの角運動量が
  `aux_bounds` の項だったため、射影が毎ステップ後に上限へ clamp する一方、body は
  そこまで運んだ反作用をすでに積分し終えている: clamp はその分だけ保存量を捨てており、
  刻みが粗いほど捨てる量が増えた。上限への到達は二分探索で時刻を求める境界になり、
  `settle_boundary` が角運動量を上限に載せ、超過分を `ω += I⁻¹ (δh a)` で body に
  返す。上限で保持されている間、ホイールは body とトルクを交換しないがジャイロ結合の項
  `−ω × H_RW` は残り、モータが要求するトルクが内向きに変わった時点で解除される。同じ
  伝播は、設定が提供する全ての積分器で合計を 1e-9 N·m·s まで保存するようになった。
  ([#446](https://github.com/sksat/orts/issues/446))
- **破壊的変更**: `SunSensor` が、noise を有効にすると単位ベクトルでない方向を返していた。
  あわせて `SunDirectionBody::new` の戻り値が `SunDirectionBody` から
  `Option<SunDirectionBody>` になった。自分で構築していた呼び出し元は、方向を持たない
  ベクトルに対する `None` を扱うか、方向であることが分かっている入力なら `.expect(...)` する。センサは
  satellite→Sun を正規化して body frame に回してから noise を足し、再正規化せずに包んでいたが、
  型と WIT はどちらも unit vector と書いてある。入力を 1.1 倍する noise model ならノルムは 1.1、
  組み込みの Gaussian noise の σ = 0.01 なら各サンプルが 1% 程度ずれる。内積を cos として読む
  guest や、成分から角度を復元する guest は誤った値を得る。`SunDirectionBody::new` が正規化を
  行う場所になり、ノルム 1 を守るのが `SunSensor` の手順ではなく型の側になった。方向を持たないベクトル
  (非有限な成分、ノルム 0 — additive noise の打ち消しで起こりうる) には `None` を返す。正規化は
  最大成分でスケールしてから行うので、二乗が overflow する成分でも単位ベクトルになる。
  `SunSensor::measure` は宇宙機が太陽中心にある場合も渡さなくなった (従来は正規化していない
  差ベクトルを、noise 倍して返していた)。`illumination` はどの場合も変えていない。これは太陽の
  幾何的な可視割合で、測定が成功したかどうかの旗ではない。したがって `direction: None` の
  2 つの理由は illumination で区別できる — 0 なら本影、正なら方向が測れなかった場合。
  この区別は WIT にも書いた。
  ([#447](https://github.com/sksat/orts/issues/447))

- `AtmosphericDrag` と `PanelDrag` の既定の `omega_body`、および
  `perturbations::OMEGA_EARTH` の re-export が `arika::earth::ERA_RATE` になった。共回転する
  大気は frame の `R` 段と一緒に回るので、その角速度は測地系の nominal 定数ではなくその段が
  進む速度である。**drag の加速度とパネルのトルクが変わる** — `AtmosphericDrag` で相対 9.46e-9、
  `PanelDrag` の立方体で 7.63e-9(snapshot の state で実測)。テストの許容は相対 1e-12 なので
  許容ではなく期待値を更新した。
- Orekit の reference fixture から `omega_earth_rad_s` の metadata field を、generator から
  その定数を落とした。この field は「Constants matched to Rust orts」という注記の下にありながら、
  Orekit の drag は co-rotation 速度を ITRF 変換から取るので、reference データが何で生成された
  かを表していなかった。`OMEGA` を訂正した以上、一致すべき Rust 側の値も無い。この field を
  読む箇所は無い。GCRF 側の generator が持っていた同じ定数(どこからも使われていなかった)も
  併せて落とした。([#480](https://github.com/sksat/orts/pull/480))
- パネル drag の加速度 snapshot 2 件が相対許容で比較するようになった。従来は
  `1e-12 * expected.magnitude().max(1.0)` で、加速度が約 1e-9 なので floor により、意図した
  相対 1e-12 が絶対値 1e-12 — 値の 7.9e-4、8 桁緩い — になっていた。ERA rate による加速度の
  変化 9.7e-18 に対して 1e5 倍の余裕があり、snapshot は通り続けていた。隣にあるトルクの
  assert は同じ理由で floor を避けていた。([#480](https://github.com/sksat/orts/pull/480))
- 積分 step より短い燃焼も伝播に入るようになった。`IndependentGroup` と `CoupledGroup` は
  現在時刻から目標時刻まで積分器を 1 回走らせていたので、隣り合う評価点の最大間隔より狭い
  `BurnWindow` が評価点の間に落ちていた。RK4 で `dt = 1` のとき `[0.1, 0.2)` は推進剤
  3.399e-4 kg を 1 つも消さない。tolerance を締めても直らない: 評価点が区間に入らないあいだ
  誤差推定は厳密に 0 で、制御器はその step を受理する。区間が step 全体を覆う場合も、
  終端に乗る stage が throttle を off と読むので推進剤が 5/6 になっていた。両方のループが
  system の報告する境界で span を区切り、segment ごとに束縛した system と新しい stepper で
  積分する。**燃焼を含む軌道と消費推進剤が変わる**。([#453](https://github.com/sksat/orts/pull/453))
- `ConstantThrust` が Δv を `[start, end]` でなく `[start, end)` に配り、積分中の segment に
  ついて答えるようになった。以前は燃焼区間が両端を数えていたので、その端で span を区切ると、
  燃焼前の segment の終端 stage と燃焼後の segment の開始 stage が両方とも推力を on と
  読んでいた。0.1 s の燃焼を RK4 `dt = 1` で伝播した実測で、指定 Δv の 4/3 倍を適用していた。
  終端を含めたままにすると segment では stage 単位より悪く、燃焼終了時刻から始まる segment が
  全長にわたって噴射する。**燃焼の最後の瞬間は噴射しなくなり**、燃焼全体の Δv が指定値に
  一致する。([#453](https://github.com/sksat/orts/pull/453))
- `PanelSrp` と `PanelDrag` が、一部だけ影に入るパネルに日向のぶんだけの力を、日向の重心で
  与えるようになった。以前はパネルごとに「全部日向」か「全部影」かを答えていたので、半分影に
  入るパネルが面全体ぶんの力を面の中心に出していた。1 m 立方の衛星本体の両側に 2 m x 1 m の
  SAP を展開した形状では、太陽が SAP の法線から 77° を超えると真値の 5.6〜9.6 倍の偽の SRP
  トルクが出ていた。影は和集合として扱うので、2 枚の和で覆われるパネルも見つかる。
  **空力トルクも同じく変わる**: 2 つのモデルが同じ幾何を共有しているので、半分が wake に入る
  パネルは露出したぶんの面積を露出部分の重心で持つ。輪郭を持たないパネルは影響を受けない。
  光源に対して edge-on から 1e-12 以内のパネルは落とす。影の射影が f64 の範囲を出る境界で、
  力も face-on の 1e-12 以下になる。([#444](https://github.com/sksat/orts/pull/444))
- WASM host の `magnetic-field-eci` が中心天体の磁場を返すようになった。sync と async の
  両 backend が host state を `TiltedDipole::earth()` 固定で作り、中心天体を見ていなかった
  ので、Mars の run でも plugin には Earth の磁場が返っていた。#372 が残した唯一の経路で、
  guest は config に磁気デバイスを書かなくてもこの import を呼べる。host も中心天体を受け取り、
  モデルがない天体では `tobari::magnetic::NoField` を使う（CLI のデバイスと同じ規則、
  `orts::magnetic::field_is_modelled`）。WIT の変更は不要で、戻り値の `vec3` にゼロを載せる。

  **BREAKING**: `WasmController::new`、`AsyncWasmController::new`、それぞれの
  `new_with_streams`、`WasmPluginCache::build_*_controller*` が `KnownBody` を取る。
  呼び出し元のなかった legacy alias `WasmPluginCache::build_controller` は削除した
  （天体を勝手に選ぶ入口を残さない）。([#431](https://github.com/sksat/orts/issues/431))

- `PanelDrag` が、風下の面ではなく流れに向いた面に抵抗を出すようになった。`v_rel` は
  大気に対する機体の速度なので、機体の系では大気は `+v_rel` の側から来る。判定式が
  `cos θ = n̂·(-v̂_rel)` だったため機体の陰にある面を選んでいた (`PanelSrp` は太陽を
  向いた面を選ぶ)。パネルの影の幾何は入射方向をモデルから受け取るので、
  影も同じく風下側に落ちていた。今は大気が来る側に落ちる。変わるのは影の側だけで、
  一部だけ隠れたパネルの日向がどれだけ残るか、その力がどこに働くかは
  [#444](https://github.com/sksat/orts/pull/444) のままである。面積と `cd` が等しい 2 面の対 (`SpacecraftShape::cube`
  が作るもの) では、力の大きさはどちらを選んでも同じになる。`cd`・面積・cos θ の積の和が
  等しいからである。`cd` と面積は面ごとに持つので、どちらかが違う対では大きさも変わる。
  どの場合も変わるのは力が通る `cp_offset` で、対になる 2 面の offset が異なる機体では
  姿勢外乱の向きが誤っていた。流れに向けた片面のパネルには抵抗が付かなかった。面の選び方と cos θ の
  法則は、Orekit の paneled drag model に対する fixture
  (`tools/generate_orekit_panel_drag_fixtures.py`) で固定した。
  ([#437](https://github.com/sksat/orts/pull/437))
- `SurfacePanel::at_com` と `SurfacePanel::rectangle` が、magnitude を計算できない
  方向ベクトルを、無限大の normal を持つ panel にせず reject するようになった。
  正規化した後で結果の長さを検査していたので、`[1e-200, 1e-200, 1e-200]` が通って
  いた: squared norm が 0 に underflow するため正規化が 0 除算になり normal が無限大
  になって、そこから計算する力はすべて NaN になる。呼び出し側が渡したベクトルの
  magnitude を検査する形にした。squared norm が overflow する `[1e300, 1e300, 0]`
  も同じ検査で落ちる。([#424](https://github.com/sksat/orts/pull/424))
- `load_as_recording` が、.rrd が entity の一部の行にしか持たない component を
  落とさず復元するようになった。列が「自分の覆う行」を持てなかった間は落とすしか
  なかった: 欠けた行をゼロで埋めると `orts convert` の CSV にファイルが持っていない
  値が入り、行を落とすと軌道が失われる。component の field が一部しか揃っていない行は
  従来どおり除外し、どの行も丸ごと持たない component は報告しない。
  ([#375](https://github.com/sksat/orts/issues/375))
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
- Sun 由来の力モデルが中心天体から幾何を取るようになった。既定の第三体 Sun と
  cannonball SRP はどちらも、中心天体に関わらず地心 Sun ベクトルを読んでいた。
  2026 年の Mars では Sun が Mars から見た方向と最大 176° ずれ、SRP は
  ベクトルとして 242〜311% 誤り（逆向きに働く）、潮汐項は最大 3.8 倍になっていた。
  SRP はさらに影の半径に Earth 半径を使っており、大きさの違う天体では食の位置が
  ずれていた。第三体重力に影はなく、誤りはベクトルだけだった。
  `ThirdBodyGravity::sun_from_body`、`SolarRadiationPressure::for_body`、
  `PanelSrp::for_body` が `KnownBody` を取り、`default_third_bodies` と 2 つの
  `build_*` builder が `Result` を返す。Sun ephemeris を持たない中心天体は、Sun を
  誤った位置に置いたまま伝播せず報告される。panel SRP も対象で、panel と Sun 方向の
  角度が力とトルクの両方を決めるため、どちらも向きが変わる。

  **BREAKING**: `SolarRadiationPressure` に private な `sun_position_fn` field が
  増える。他の field は `pub` なので downstream は struct literal で直接構築できて
  いたが、それがコンパイルできなくなる。`for_earth` / `for_body` と `with_*`
  builder を使う（crate 自身は元からこの経路で構築している）。`PanelSrp` と
  `SunSensor` も同じ field を持つが、こちらの field は元から private。([#372](https://github.com/sksat/orts/pull/372))
- sun sensor の Sun 方向も中心天体から取るようになった (`SunSensor::for_body`)。
  地心ベクトルを読んでいたため、Mars では力モデルが正しくても姿勢制御の入力だけが
  最大 176° ずれていた。**CLI のセンサが食を見るようになる**: これまでは
  `SunSensor::new()` で組んでいて遮蔽天体を持たなかったので、地球の裏側でも
  日照として読めていた。`for_body` は中心天体の円錐影を入れる。地球の真裏
  7000 km での実測は `direction: Some(..), illumination: 1.0` から
  `direction: None, illumination: 0.0` へ変わる。direction を unwrap していた
  controller plugin は不在を扱う必要がある。
  ([#372](https://github.com/sksat/orts/pull/372))
- Sun 中心の伝播でも SRP は残る。Sun が原点なので衛星→Sun ベクトルは `-r_sat` で、
  遮るものもない。落ちるのは第三体項だけ。([#372](https://github.com/sksat/orts/pull/372))
- 中心天体から見た Sun ベクトルを Orekit の DE ephemeris と突き合わせるようにした
  (`arika/tests/oracle_body_sun.rs`、fixture は
  `tools/generate_body_sun_fixtures.py` が生成)。7 天体 × 2000-2075 年の 6 epoch で、
  方向と距離の両方を比較する。arika はこのベクトルを Earth では Meeus 級数、Moon では
  そこから月ベクトルを引いた差、惑星では Standish の要素から作っており、既存のテストは
  そのうちの一つを別の一つと比べていた。符号・位相・黄道→赤道の誤りは全部を同時に
  すり抜けうる。一致は Earth・Moon・Mercury・Venus が 0.4' 以内、Mars が 1.2'、
  Jupiter が 6.5'、Saturn が 16.9'。黄道を回さなければ 23.4° ずれる。
  ([#372](https://github.com/sksat/orts/pull/372))
- `PanelSrp::without_shadow()` を追加。`SolarRadiationPressure::without_shadow()`
  と同じく、影だけを外して構築時の Sun 方向を保つ。影なしの panel SRP は
  `PanelSrp::new` からしか作れず、そちらは地球の Sun を読むので、他の天体では
  照らす面を間違えていた。2026-03-20 の Mars の Sun 方向は地球のそれと 152.8° 離れ、
  Mars の Sun を向けた panel は `new` 経由では力が厳密に 0 になる。
  ([#372](https://github.com/sksat/orts/pull/372))
- 磁場がモデル化されていない天体では、磁気デバイスが Earth の磁場ではなくゼロを読むように
  なった。`Igrf` と `TiltedDipole` は Earth のもので、磁場モデルはこの 2 つしかない。従来は
  Mars を回る衛星が Earth の磁場を読み、そこからトルクを作っていた。`tobari::magnetic::NoField`
  を追加し、magnetometer の読みは 0、magnetorquer の `m × B` も 0 になる。デバイス自体は
  どちらの天体でも構築されるので、同じ衛星定義をどの天体にも向けられる。`orts run` は
  デバイスごとに 1 回、天体名を含む warning を出す。WASM host の `magnetic-field-eci` は
  Earth の磁場を返すまま ([#431](https://github.com/sksat/orts/issues/431))。
  ([#372](https://github.com/sksat/orts/pull/372))
- Moon 中心の伝播に第三体として Earth が入るようになった (Moon の第三体環境で
  支配的だが欠けていた)。Sun 中心の伝播は Sun を自分の周回衛星に対する第三体として
  足さなくなった。([#372](https://github.com/sksat/orts/pull/372))
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
- SRP shadow model が中心天体の半径を使うように修正。`build_orbital_system` /
  `build_spacecraft_dynamics` は `for_earth` で SRP を構築しており、shadow
  半径が中心天体によらず地球の 6378 km だった — Moon 中心のシミュレーション
  では shadow cylinder が約 3.7 倍広く、衛星を誤って eclipse 判定し SRP が
  過小適用されていた。両 builder が共通の `build_srp` helper 経由で
  `BodyProperties::radius` に shadow をサイズするようになった。
  ([#385](https://github.com/sksat/orts/pull/385))

#### Removed
- **BREAKING**: `orts::tle` module を削除。TLE パースは `arika::tle`
  (共有 `arika::elements::Sgp4Elements` へデコード) に移管。`orts::tle` を使う
  下流コードは `arika` へ移行が必要。([#87](https://github.com/sksat/orts/pull/87))

### `orts-cli` (Rust, crates.io, binary)

#### Changed
- **BREAKING**: `serve` は、WebSocket の `start_simulation` に書かれた `space_weather` の
  ファイルパスを、`[gravity_field]` と同じく拒否する。client は指定しないか、CelesTrak から
  取得する `"auto"` を指定する。パスは server のファイルを指し、manager がそれを読んでいた —
  FIFO で測ると読み込みが戻らず、server は新しい WebSocket の handshake を完了しなくなった。
  `--config` のファイルと `--space-weather <PATH>` は、server を起動する人が指定するので、
  これまでどおりパスを受け付ける ([#556](https://github.com/sksat/orts/issues/556))
- **BREAKING**: `run` と `serve` は、simulation が読まないコマンドラインのフラグを拒否する。コマンドは
  simulation を config (`--config`、または引数なしの `run` が見つける `orts.toml`) か、コマンドラインの
  軌道指定から取り、調整フラグ (`--dt` / `--atol` / `--integrator` / `--duration` / `--frame` /
  `--gravity-field` など) は後者にだけ効く。`run --config mission.toml --dt 1` は config の `dt` で
  黙って積分していた — 固定刻みの run で、フラグの有無で出力が最終桁まで一致した。いまは clap が
  usage エラー (exit 2) で止める: `--config` と並んだ調整フラグ・軌道指定
  (`the argument '--config <CONFIG>' cannot be used with '--dt <DT>'`) と、軌道指定のない調整フラグ
  (`the following required arguments were not provided: <--sat …>`)。軌道を黙って捨てていた
  `serve --config` + `--sat` と、見つけた `orts.toml` の上に書いた `run --dt 1` もこれで止まる。
  フラグは `SimArgs` に引数の関係として宣言したので、値によらず「書かれたこと」で数える。`serve` が
  持っていた手作りの判定 (`WrittenFlags` とフラグ表) は無くなった。捨てられるフラグが複数あるとき、
  エラーが全部を名指しするのは `--config` が先にある場合だけである: clap は最初に見つけた衝突を
  報告し、残りは Usage 行に出る。`--plugin-backend` の 3 つはどちらのコマンドでも適用されるので
  影響しない。config への移し方: `--dt` / `--duration` / `--output-interval` / `--epoch` は同名の
  トップレベルのキー、`--integrator` / `--atol` / `--rtol` / `--root-t-tolerance` は `[integrator]`
  (integrator 自体は `type`)、重力のフラグは `[gravity_field]` (`path` / `degree` / `order`)、
  `--sat altitude=400` は `orbit = { type = "circular", altitude = 400 }` を持つ `[[satellites]]`。
  config には OMM ファイルを指すキーが無く、TLE は 2 行を書き写す形になるので、それらは
  コマンドラインで走らせる ([#550](https://github.com/sksat/orts/issues/550))
- `--tle-line1` / `--tle-line2` の片方だけ、`--gravity-field` のない `--gravity-degree` /
  `--gravity-order` は usage エラー (exit 2) になる。TLE の行が片方だけだと
  `SimParams::from_sim_args` が panic していた ([#550](https://github.com/sksat/orts/issues/550))
- **BREAKING**: `serve` は、値が既定と違うからではなく「書かれたから」調整フラグを報告する。
  `serve --config mission.toml --atol 1e-10` は無言で起動していた: 値が既定と同じなのでフラグが
  「無い」と読まれ、コマンドラインの指定ではなく config の `atol` が走っていた。この検査が走るのは
  これらの値を誰も読まない経路だけである — config は `SimParams::from_config` が組み、idle server は
  クライアントの `start_simulation` から取る — ので、書かれた値は何であれ落ちる。既定値と一致して
  いたために受け付けられていたコマンドは、フラグ名を挙げて拒否されるようになった
  ([#522](https://github.com/sksat/orts/issues/522))
- `serve` が plugin を `--plugin-backend-async-mode` の指定したモードで走らせる。client が
  あとから始めた fleet にも適用される: このフラグは `PluginBackendOverrides` に載って manager が
  組むすべての `SimParams` に入り、`ServeEngine` は plugin cache を `WasmPluginCache::new()`
  (モードは常に `Deterministic`) ではなくそのモードで作る。フラグを指定しない場合、`serve` は
  `Deterministic` のままである — 指定のない server が従来やってきたことなので、このフラグを書かない
  コマンドラインの挙動は変わらない。2 つのコマンドで既定が違うので、このフラグは clap の既定値を
  持たない: 指定しなければ `run` は `throughput`、`serve` は `deterministic` を使う。
  `serve` で `throughput` が得るのは multi-worker runtime までで、step は衛星を順に回すため
  制御ステップは `run` のように重ならない ([#548](https://github.com/sksat/orts/issues/548))
  ([#536](https://github.com/sksat/orts/issues/536))

#### Added
- `frame` / `--frame {simple-eci|gcrs}` と `eop` / `--eop {auto|PATH|zero}`:
  `orts run` の軌道のみの経路を `Gcrs` (IAU 2006/2000A CIO chain + 観測 IERS
  EOP、極運動込み) で伝播できるようにした。従来の ERA のみの `SimpleEci` が既定。
  `auto` は IERS から `finals2000A.all` を取得 (24h キャッシュ)、パス指定はローカル
  ファイル、`zero` は model CIP のみ。`gcrs` は Earth 専用・EOP 必須で、姿勢付き
  fleet・コントローラ・`orts serve` (いずれも `SimpleEci` 固定) では黙って fallback
  せず reject する。`auto` またはファイル指定では、伝播区間が EOP table の範囲を
  外れる run を伝播前に reject し、table と run の MJD 範囲を示す (範囲外では
  transform が末尾の行を保持して黙って精度を落とすが、それは `zero` だけが明示的に
  選ぶもの)。recording の metadata に frame を残し (`# frame = gcrs`)、`orts replay`
  は `simple-eci` 以外の frame で伝播した recording を reject する (viewer は描く
  すべての state に ERA のみの地球回転を掛けるので、黙って誤った ground track を
  出すことになる)。([#411](https://github.com/sksat/orts/issues/411))
- `[gravity_field]` config table (`path`, `degree`, `order`) と `run` / `serve` の
  `--gravity-field <PATH> [--gravity-degree N] [--gravity-order M]`: ICGEM `.gfc`
  の完全球面調和重力場を J2/J3/J4 の zonal model の代わりに登録し、ファイルの GM を
  simulation の μ (初期状態、energy 派生値、CSV/RRD metadata、WS `info`) に使う。
  Earth 専用。`degree >= 2`、`order <= degree` は `config validate` で検査し、
  ファイルは実行時に開く。WebSocket の `start_simulation` では受け付けない
  (server 側のファイルを指すため)。伝播 frame は `SimpleEci` のままなので、場の
  回転は ERA のみ。([#411](https://github.com/sksat/orts/issues/411))
- `[[satellites.panels]]` で衛星に平板の外形を与えられるようにした
  (`area` または `half_extent` + `in_plane_x` ([#424](https://github.com/sksat/orts/pull/424))、`normal`, `cd`,
  `specular`, `diffuse`, `cp_offset`, `two_sided`
  ([#399](https://github.com/sksat/orts/pull/399))、`back` ([#395](https://github.com/sksat/orts/pull/395)))。
  `half_extent` を書くとパネルに輪郭ができ、別のパネルに完全に覆われていることを
  判定できるようになる。`area` だけのパネルは、照らされたときの力は同じだが遮蔽の
  判定には関与しない。
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

#### Added
- `orts serve` が、モデルごとの body torque を wire に載せるようになった。`accelerations` の
  隣に `torques: [{ model, torque_body_nm }]` として送り、history segment にも保存するので、
  再生した区間も live と同じものが読める。map ではなく list なのは `Model::name` が一意でない
  ためで、同名を答えるモデルが 2 つあると map は後のものだけを残す。名前はモデル自身のもので、
  `accelerations` がパネルモデルを力の名前に読み替えるのとは違う。トルクはモデルごとに報告する
  値で、どのモデルが出したかが読み手の確認したいことである。

  controlled の衛星は加速度も報告するようになった (これまでは何も報告していなかった)。実行中に
  追加した衛星も、2 つ目のサンプルからではなく最初のサンプルから両方を報告する。controlled は
  group に move する前に自分の dynamics から、orbit-only は以降のサンプルと同じ snapshot から
  読む。`orts replay`
  で `.rrd` を再生した場合のトルクはまだ無い。`RrdRow` が `orts run` の書くモデルごとの列を
  読まず、再生は perturbations を空で通知するので、値があってもチャートが出ない。
  ([#470](https://github.com/sksat/orts/pull/470))
- `orts run` が、モデルごとの外乱トルクを CSV と `.rrd` に出すようになった。1 モデル 1 列で
  `gravity_gradient.torque_body_x_Nm` のような列名になり、値は機体座標系 [N·m]。これまでは
  トルクに関する値が library から出ていなかったので、パネルモデルが出す SRP と空力の
  トルクは Rust のテストを書く以外に確認できなかった。外乱が効いていることの手がかりは
  出てくる姿勢だけで、どのモデルがどれだけ出しているかを分ける出力はなかった。plugin
  controller のある経路とない経路の両方で記録する。

  モデルは各サンプル自身の `(t, state)` で評価し直す。積分器の評価を流用しない。RK の
  内部段の時刻であり、適応刻みは棄却したステップも評価し、`EvalSegment` を読むモデルは
  境界で意図的に別の値を返すからである。コストは 1 サンプルあたり導関数 1 回ぶんで、
  パネル 22 枚の機体で 50.1 µs (導関数は 50.5 µs) を実測した。RK4 で `output_interval` が
  `dt` と同じなら、モデル評価の作業が 4 分の 1 ほど増える。([#466](https://github.com/sksat/orts/pull/466))

#### Fixed
- `run` は出力時刻ごとに 1 行だけ書く。出力間隔が小数だと、CSV の最後の行が 2 回出ていた:
  `--dt 0.2 --duration 3600` で 18002 行になり、最後の 2 行がどちらも `3600.000` だった。出力
  時刻は間隔の足し算で作っていて、0.2 を 18000 回足すと終端の 3600 s より 1.08e-9 小さい。run が
  終わったとみなす 1e-9 の範囲の外なので、その時刻と終端の 2 つを記録していた。いまは出力時刻を
  controlled な run と同じく `n × 間隔` で数え、終端の丸め 1 回分だけ下に来た時刻 (1e-9 以内、
  大きい時刻では終端の数 ulp 以内) は終端そのものにする。controlled な run も、最後の出力境界が
  終端のすぐ下に来ると同じく終端を 2 回書いていた — `--output-interval 0.3 --duration 0.9` では
  `3 × 0.3 = 0.8999999999999999` で記録したあと、ループの後で 0.9 を記録し、どちらも `0.900` と
  出た。いまは 0.9 で 1 回だけ記録する
  ([#562](https://github.com/sksat/orts/issues/562))
- `run` と `serve` は、1 つのコマンドラインに書かれた 2 つの軌道を usage エラー (exit 2) で
  止め、両方のフラグを名指しする: `the argument '--sat <SATS>' cannot be used with
  '--norad-id <NORAD_ID>'`。以前は `SimParams::from_sim_args` が panic していた (exit 101)。
  `--sat` / `--tle` / `--omm` / `--norad-id` と、`--tle-line1` / `--tle-line2` の組のうち、
  どの 2 つを組み合わせても止まる ([#551](https://github.com/sksat/orts/issues/551))
- `run` と `serve` は、軌道や space weather の入力を読めない・解析できない・取得できない
  とき、`Error:` を出して exit 1 で止まる。以前は panic していた (exit 101): `--tle` /
  `--omm` のファイルが無い・壊れている、`--tle-line1` / `--tle-line2` が壊れている、
  `--sat` の値が解析できない・知らないキーがある、`--body` が知らない天体である、NORAD id の
  TLE をどの取得元も返さない (`--norad-id`、`--sat norad-id=`、config の `type = "norad"` の
  軌道)、`space_weather` のファイルが無い、`auto` の取得に失敗した、の各場合。`serve` では
  WebSocket client も同じ panic を起こせ、その後 server は再起動するまですべての接続を閉じて
  いた: server が取得できない NORAD の衛星や、取得に失敗する `space_weather = "auto"` を
  指定した `start_simulation` と、壊れた TLE の `add_satellite` で、後者は走っていた
  simulation も失っていた。いまは server が動き続け、NORAD と TLE の要求には client に
  エラーが返る。`"auto"` の取得は要求に応答した後に行うので、その失敗はまだ server の
  stderr にしか出ない
  ([#555](https://github.com/sksat/orts/issues/555))
  ([#554](https://github.com/sksat/orts/issues/554))
- `--sat` は、読まない部分を含む spec を拒否する。以前は別の軌道で exit 0 のまま走っていた:
  `altitude800` (`=` の書き忘れ) は既定の 400 km、`altitude=800,inclination51.6` は赤道軌道、
  `tle-line1` だけでは 400 km の円軌道。同じキーの 2 回目と、1 つの spec に 2 種類の軌道の
  キーがある場合 (TLE の行と `altitude` では `altitude` が捨てられていた) も拒否する。config の
  `orbit = { type = ... }` と同じく、1 つの衛星は 1 つの軌道を持つ。`orts run --help` の `--sat` の説明に全部の
  キーを載せた ([#558](https://github.com/sksat/orts/issues/558))
- 飽和した reaction wheel が角運動量を失う問題 (上の `orts` を参照) は
  `mode = "controlled"` の経路 — `orts run --controller` と `orts serve` — でも
  起きていた。この経路は `advance_to` で刻んでおり、止まる境界を持っていなかった。
  group と同じ walk を通るようにし、申告された境界ごとの guard を span の segment を
  越えて保持する。
- `--duration` を省略した `mode = "controlled"` の run が、config に最初に書かれた衛星の軌道周期
  ぶんで終わっていた。周期 5500 s と 7000 s の 2 機なら、記述順だけで run の長さが 5500 s か
  7000 s に変わる。fleet は時計を共有するので、「一周」は最長周期にした。全機に少なくとも 1 周期ぶんの
  時間を与える最短の horizon である。orbit-only と spacecraft は従来どおり衛星ごとに自分の周期で
  終わる。正で有限でない周期は飛ばし、使える周期が残らなければ従来の 3600 s になる。どちらも
  その衛星を覆う horizon ではなく、run が使えない数値に対する guard である。受理される config から
  到達する: 円軌道の検証は高度が有限で `radius + altitude > 0` であることだけを見るが、周期
  `2 pi sqrt(r0^3 / mu)` は `r0 = 1e103` で `r0^3` が overflow して inf になる。導出した時点で
  こうした周期を拒否して全モードを揃える件は #492。
  ([#442](https://github.com/sksat/orts/issues/442))
- `mode = "controlled"` が、controller が `output_interval` より遅いと `output_interval` を
  無視していた。span の終端が controller tick か `duration` だけだったので、controller period 1 s
  に対して `output_interval = 0.1` と書いてもサンプルは 1 s ごとで、しかも記録される時刻は出力
  境界ではなく tick の時刻だった。`next_output_t` も 1 回の発火で 1 回しか進まないため、tick ごとに
  `t` から離れていった。span は「次の fleet event・次の出力境界・`duration`」の最も早いところで
  終わるようにし、出力境界は累積加算でなく counter による `n * interval` で置いた(累積は 6 回目から
  倍数を下回る)。
  ([#442](https://github.com/sksat/orts/issues/442))
- `mode = "controlled"` が、地表に衝突した衛星や大気圏に入った衛星を止めていなかった。
  orbit-only と spacecraft は `body_event_checker` を `IndependentGroup` に渡すが、controlled の
  ループは終了判定なしで積分していた(adaptive 側は `Continue` 固定の closure、`Rk4` は hook のない
  `try_integrate`)。地表以下の衛星が伝播され続け、`serve` の終了判定も構造上 `false` を返していた。
  3 つの integrator をすべて `stepper().advance_to(..., event_check)` に通した。目標時刻に達したら
  その時刻を、event なら stepper が止まった状態と時刻を commit し、積分エラーでは従来どおり
  衛星を元の位置に残す。検査は controller より先に行うので、衛星が止まった時刻に予定されていた
  tick は走らず、地表下で投入された衛星は一度も積分されない。1 機の終了で fleet の run は
  終わらない。`run` はその衛星を自身の終了時刻で 1 度記録し、orbit-only と同じ形で報告して、
  以降のサンプルと可視性の評価から外す。`serve` は既存の broadcast と再接続用リングに流れる経路で
  `simulation_terminated` を送る。
  ([#442](https://github.com/sksat/orts/issues/442))
- 同じ RRD に対して `orts replay` を 2 回実行すると、viewer に送るメッセージの中身が違っていた。
  読み込んだ状態を entity path ごとに `HashMap` でまとめていたためで、その反復順に従っていたのは次の 3 つ。`Info` メッセージの衛星の
  並び、median のサンプル間隔を `dt` として採る entity、そして同じ時刻を持つサンプル同士の順序 —
  overview・`all_states`・`query_range` の応答はいずれも entity ごとの列を統合してから `t` で
  安定ソートするので、同時刻の中では反復順がそのまま残る。`BTreeMap` に替えて、どれも entity
  path の順になった。
  ([#441](https://github.com/sksat/orts/issues/441))
- `orts serve` が、実行中に追加した衛星の持つモデルを報告するようにした。
  `satellite_added` は `perturbations` を空で通知していて、保持している Info
  (追加後に接続したクライアントに送るもの) も起動時の snapshot のままだったため、
  追加した衛星がそこに存在しなかった。両方の追加経路で、構築した系から名前を読み、
  その snapshot に衛星を記録する。
  ([#474](https://github.com/sksat/orts/pull/474))
- CSV の全衛星の行が、header が名前を挙げた列を持つようになった。header は先頭の衛星の
  component から作られ、各衛星の行はその衛星自身の component から書かれていたので、
  衛星ごとに記録している component が違うと行の列数が揃わなかった。2 機のうち先頭だけが magnetorquer の指令を
  持つ場合、header は 17 列で 2 機目の行は 14 列になり、CSV として読み戻せない。
  その CSV に出てくる全 component を合わせた列のリストを 1 度作り、header と全衛星の行が
  同じリストを歩くようにした。
  その component を持たない衛星は欄を空にする (ステップで値が欠けたときに既にそうしていた
  のと同じ扱い)。列名は recording の component registry から取る。log と `.rrd` の
  読み込みのどちらも registry を埋める。([#465](https://github.com/sksat/orts/pull/465))
- `--tle <path>` と `orbit.tle` が、2 衛星以上を含むファイルを拒否するようになった。
  複数衛星のカタログは先頭の 1 衛星だけを伝播し、残りについて何も言わなかった。今はファイルの
  行数を示して停止する。カタログを分割し、1 回の実行につき 1 衛星を渡すこと。([#462](https://github.com/sksat/orts/pull/462))
- controlled loop が、積分 step より短い燃焼も伝播に入れるようになった。
  `propagate_controlled` は span の始まりから終わりまで積分器を 1 回走らせていたので、schedule を
  持つ dynamics では #453 以前の group ループと同じ取りこぼしが起きていた。`ScheduledBurn` で
  `[0.15, 0.25)` を指定し RK4 `dt = 1` で伝播した実測で、燃焼が要求する推進剤 3.399e-4 kg を
  1 つも消さない。dynamics が報告する境界でループを区切り、segment を束縛した system で積分する
  ようにした。非有限の span は「すでに終わっている」ではなく拒否する (`t1 <= t0` も、ループ自身の
  条件も NaN では偽になる)。CLI の config から構築されるもので境界を宣言するものは今はないので、
  影響を受けるのは `ControlledSatellite` を自分で組む呼び出し元。([#455](https://github.com/sksat/orts/pull/455))
- `duration` が各衛星の軌道周期を置き換えなくなった。`duration` は run の終了時刻だが
  両方が 1 つの field に載っていたため、`--duration 120` は CSV header・RRD の
  `meta/sim/period`・WebSocket の `SatelliteInfo` のすべてに、5553.6 s かかる軌道の
  周期として 120 s を載せていた。全衛星に同じ duration が入るので fleet の周期も
  1 つの値に潰れていた。horizon が必要な箇所は `duration.unwrap_or(period)` から
  終了時刻を取る。`duration` 未指定なら、orbit-only と spacecraft の run は従来どおり
  各衛星の 1 周分を回り、controlled の run は従来どおり最初の衛星の周期を fleet 全体の
  終了時刻にする。([#368](https://github.com/sksat/orts/pull/368))
- 走っている `orts serve` に追加した衛星が、自分自身の周期で軌道を再設定するように
  なった。`serve` は無摂動の orbit-only 衛星を周期の境界ごとに初期軌道へ戻すが、
  新規追加した衛星の境界は fleet の**直前**のエントリから読んでいた (最初の追加では
  5554 s のハードコード)。([#368](https://github.com/sksat/orts/pull/368))
- 各 controller が自分の `sample_period` で動くようになった。2 つの loop がこれを
  動かしていた。非 realtime の `orts serve` は `stream_interval` で timeline を切り、
  切るたびに controller を 1 回呼んでいた。README quick start の config は
  `output_interval` と `stream_interval` を `dt = 0.01` のままにし、`pd-rw-control` は
  0.1 s を要求するので、10 Hz の controller が 100 Hz で回り、各コマンドの保持時間が
  意図の 1/10 になっていた (実測: sim 時間 1 s あたり 10 回でなく 100 回)。`orts run` は
  fleet の最短周期で全衛星を回すので、0.1 s の controller と並ぶ 1.0 s の controller が
  毎秒 10 回 tick していた。どちらの loop も、保持中のコマンドのまま必要な境界まで
  積分し、その時刻に due な衛星だけを tick する。
  ([#369](https://github.com/sksat/orts/pull/369))
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

#### Fixed
- `detumble-nadir` の Nadir モードが nadir を指向するようになった。star tracker の
  body→inertial 姿勢をそのまま誤差クォータニオンとして扱っていたため、目標は
  慣性 identity 姿勢・目標角速度 0 だった。`input.spacecraft.orbit` を参照せず、
  LVLH 目標も軌道角速度 (LEO で ~1.1e-3 rad/s) も計算していない。nadir 指向が
  成立している状態に対して 1.0 N·m の wheel torque を指令し、その間
  `current_mode()` は "nadir" を返していた。([#367](https://github.com/sksat/orts/pull/367))
- 指令を組めない tick が、そのモードが駆動するアクチュエータに明示的なゼロを
  指令するようになった。WIT の `command` 契約では `None` は「コマンドなし =
  前回値を保持」でゼロではないため、detumble 最後の磁気モーメントが nadir フェーズ中
  ずっと通電し続け、センサ欠損時には wheel torque 指令が残って wheel が飽和へ
  加速し続けていた。([#367](https://github.com/sksat/orts/pull/367))
- `transfer-burn-with-tcm` と `constellation-phasing` が apogee で円化するように
  なった。transfer ellipse の半周期は *perigee* からの時間だが、両 example は
  そのタイマーを有限時間の first burn の**終了時**から起動していたため、burn が
  張った弧の分だけ apogee を過ぎてから点火していた。同梱の 5000 N / 500 kg では
  1.66° のずれで、推力が下がるほど弧が広がりずれは無限に大きくなりうる。apogee は
  径方向速度 `r·v` の符号反転で検出し、タイマーは watchdog に降格した。
  ([#367](https://github.com/sksat/orts/pull/367))
- 両 example の同梱設定・bench スクリプト・README が `target/wasm32-wasip2/` を
  指していた。host が読むのは component で、それを出す `cargo component build` の
  出力先は `wasm32-wasip1` なので、手順どおりに実行しても plugin が見つからなかった。
  `detumble-nadir` にはそもそも設定が無く一度も end-to-end で動かされていなかったので、
  追加した。([#367](https://github.com/sksat/orts/pull/367))

### `arika` (Rust, crates.io)

#### Added
- `fetch-eop` feature: `EopTable::fetch` / `fetch_default` が IERS の
  `finals2000A.all` を取得し `~/.cache/orts/finals2000A.all` に 24h キャッシュする
  (`CssiSpaceWeather::fetch` と同じ作り)。`ClampedEop::new` が `Borrow<EopTable>`
  を受けるので、1 つの table を複数の力学モデルで共有できる。
  ([#411](https://github.com/sksat/orts/issues/411))

#### Added
- `arika_wasm::orbit_derived_batch` が、状態ベクトルの配列から Kepler 要素と
  軌道のスカラー量を返すようになった。ブラウザが `.rrd` を読むときも、CLI が CSV に
  書くのと同じ `KeplerianElements::from_state_vector` で計算される。軌道面を持たない
  状態 (`r = 0` や `r × v = 0`、後者は `v = 0` を含む) は 0 ではなく `NaN` を返す。
  0 は円赤道軌道の角度として実在する値なので、値なしの印には使えない。
  ([#376](https://github.com/sksat/orts/pull/376))
- `arika::sun::sun_position_from_body` が `KnownBody` を取り `Result` を返すように
  なった。既存の `sun_direction_from_body` は天体を `&str` で取り、認識できない天体に
  +X 方向の単位ベクトル (春分点方向で、どこから見ても Sun の方向ではない) を返す。
  この crate は Mercury〜Saturn の Standish 要素しか持たないため Uranus と Neptune が
  その分岐に入る。新しい関数はこの 2 天体と、中心天体が Sun の場合を拒否する。
  ([#372](https://github.com/sksat/orts/pull/372))
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
- `Finals2000A::parse` (したがって `EopTable::from_finals2000a` / `fetch`) が配布
  されている `finals2000A.all` を読めるようにした。ファイル末尾には日付だけあって
  EOP 列がすべて空の行が約 50 行 (全幅に空白詰め) 並んでおり、parser はその最初の
  行を `xp_A` の不正な数値として全体を失敗させていた。そのため `--eop auto` は
  download 成功直後に失敗し、切り詰めた test fixture では見えなかった。Bulletin A
  の必須値がすべて空の行は系列の終端として skip し、一部だけ空の行は従来どおり
  エラー。実ファイルの末尾から切り出した fixture で固定。
  ([#411](https://github.com/sksat/orts/issues/411))
- `tle::parse` が、1 衛星ぶんより多くの行を含む入力を拒否するようになった
  (新しい `TleParseError::TrailingLines`)。従来は先頭のレコードを読んで残りを黙って捨てて
  いた。checksum は行ごとの mod-10 なので読んだ 2 行だけで成立し、catalog number の一致検査も
  その 2 行の間だけで比べるため、3 行目以降を見る検査が無かった — 2 衛星のカタログが先頭の
  1 衛星として通っていた。空行と末尾の空白は、従来どおり数える前に除かれる。([#462](https://github.com/sksat/orts/pull/462))
- OMM の 3 形式 (JSON / KVN / XML) すべてで `BSTAR` を必須にした。従来は欠落を `0.0` と
  読んでいたので、この field が無い OMM は抗力のない衛星として伝播し、成功として
  報告されていた。他の要素はすべて既に必須である。`0.0` が明示的に書かれている場合は
  従来どおり受理する (高い軌道では正当な値のため)。parser は `MEAN_ELEMENT_THEORY` が
  SGP4 以外を拒否するので、これは SGP4 自身が読む抗力項である。([#462](https://github.com/sksat/orts/pull/462))
- `EopTable::new` が doc の要求する順序を検査するようになった。従来は「sorted な entries」と
  書いて `Result` を返しながら、空かどうかだけを見ていた。MJD 60000, 60002, 60001 の entries が
  テーブルを作れてしまい、`mjd_range` は `(60000, 60001)` を報告する — 60002 がテーブル内に
  あるのに 60001.5 の照会は `OutOfRange` になり、lookup の `partition_point` は順序を仮定できない
  データを二分探索していた。新しい `EopLookupError::NonMonotonicMjd` で報告する。これは
  finals2000A の parser が既に `EopParseError::NonMonotonicMjd` として報告していた条件と同じ
  名前である。同一 MJD の 2 entries も、補間の区間が作れないので拒否する。([#463](https://github.com/sksat/orts/pull/463))
- finals2000A の optional 列 (LOD・dX・dY・Bulletin B の一群) が、空欄と破損値を区別する
  ようになった。従来はどちらも「欠損」として読み、欠損した補正は `0.0` で答えられる — つまり
  `0.3O0` を含む行が「IERS はここに補正を出していない」になり、何も報告されなかった。必須列は
  以前からエラーである。空欄は従来どおり欠損として扱う (IERS は予測部の補正列を空にするので、
  それはデータである)。([#463](https://github.com/sksat/orts/pull/463))
- `EopTable::from_finals2000a` が、テーブルが拒否した理由を `EopParseError::Empty` に
  潰さず報告するようになり、`Finals2000A::parse` は非有限の MJD をその行番号とともに拒否する
  ようになった。`NaN` は数値として parse され、大小どちらの比較も false になるので、parser の
  `mjd <= previous` では見えなかった。61 行のうち 1 行の MJD が `NaN` のファイルが
  `Empty` (「有効な entry を含まない」) として返っていたが、実際は 61 行あった。([#463](https://github.com/sksat/orts/pull/463))
- finals2000A の行が固定幅の列の途中で終わっている場合、その列に値は無いものとして扱う。
  従来は slice を行の長さに切り詰めていたので、先頭部分が短い数値として読まれていた。fixture の
  LOD 列は `  0.1623` で、4 文字目の後で切れた行は `  0.16` を読む — 行が 0.1623 s と書いて
  いるところで 0.16 s になる。([#463](https://github.com/sksat/orts/pull/463))
- `EopTable::new` が、増加順だけでなく全 MJD の有限性を要求するようになった。増加順だけでは
  `-inf` が有限の MJD の前に通り、テーブルは範囲 `(-inf, 60000)` を報告したうえで、その範囲内の
  照会に対しエラーではなく `Ok(NaN)` を返していた。単独の `NaN` entry は比較する相手が無いので
  そもそも検査されなかった。`EopLookupError::NonFiniteMjd` で報告する。([#463](https://github.com/sksat/orts/pull/463))
- finals2000A の全列が `NaN` / `inf` / `-inf` を拒否するようになった。`f64::from_str` は
  3 つとも受理し、IERS はいずれも公開しないので、その綴りがそのまま照会の答えに入っていた。
  `lod_A = NaN` は `Ok(NaN)` として返り、Bulletin B の `NaN` は有効な Bulletin A の値に
  優先する (B 列があるときは B を採るため)。Bulletin B の一群が無い行では、必須列も同じ経路で
  答えに届いていた。([#463](https://github.com/sksat/orts/pull/463))
- **BREAKING**: `EopParseError` と `EopLookupError` が `#[non_exhaustive]` になった。網羅的な
  `match` には wildcard arm が必要である。今回どちらも variant が増え、未着手の EOP の作業でも
  さらに増える。([#463](https://github.com/sksat/orts/pull/463))
- `earth::OMEGA` を WGS-84 の nominal mean angular velocity `7.292115e-5` rad/s に訂正し、
  Earth Rotation Angle が進む速度 `2π × 1.00273781191135448 / 86400` を
  `earth::ERA_RATE` として追加した。**`OMEGA` の値が変わる**: 従来の `7.2921159e-5` は、doc が
  引用していた IERS 2010 の値でも、IAU 2006 chain が回る速度でもなく、歳差する春分点に対する
  自転速度だった。`EarthFixedTransform` は速度変換に `ERA_RATE` を使う — 微分する `R` 段の
  速度がそれだからである。実測すると、`OMEGA` が従来持っていた値と `ERA_RATE` は `7.53e-12` rad/s
  違い、state transform の snapshot の自転軸垂直半径 6403 km では 0.0482 mm/s に相当する。
  訂正後の `OMEGA` と `ERA_RATE` の差は `1.47e-12` rad/s である。どちらの定数も地球の自転項だけを表す:
  `W·R·Q` 全体の微分は `Q̇` / `Ẇ` と LOD 補正も持つが、この変換はそれらを従来から含んでいない。
  `OMEGA` は `MU` や `R` と並ぶ測地系の定数として残る。([#480](https://github.com/sksat/orts/pull/480))
- `era_formula` のコメントが、arika の係数は canonical な SOFA 値 `1.00273781191135448` と
  約 1 ULP 違い、canonical 値は後のフェーズで採用すると書いていた。どちらの書き方も同じ f64
  (`0x3ff00b36cdc9f32b`) に丸まるので、採用すべきものは無かった。([#480](https://github.com/sksat/orts/pull/480))
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
- `RootEvent::leaves_zero_towards` が、step の開始値が 0 のときに「値がどちら向きに 0 を離れるか」を
  event に尋ねるようになった。そういう step の両端は、0 からまっすぐ落ちる場合と同じ組になるので、
  何かが向きを変えたかを言えない: $g(t) = t(0.5-t)$ を $[0,1]$ で見ると 0 から正に出て $0.5$ で正から
  負に変わり $-0.5$ で終わるが、$g(t) = -0.5t$ は 0 から負に出て同じ両端になる。答えを読むのは
  `Crossing::Reversal` だけで、その符号を開始側の代わりに使うと、あとの交差が向きの変化として
  数えられる。答えは、境界の上を動いている値が同じ root を再び報告しないための guard より先に読む
  (向きを答える event は、その値が 0 を離れると言っているためである)。実現トルクが 0 の reaction wheel は
  指令で答える (そこでのトルクの変化率は $\tau_{cmd}/T$ である)。既定の `None` は従来の読み方
  (その 0 から始まる step は何も報告しない) を保つ。
- **Breaking:** `Crossing::Reversal`。margin が尽きる event ではなく、量が向きを変えることを表す
  event 用である。`Crossing` は公開されていて `#[non_exhaustive]` でもないので、variant を網羅的に
  `match` している側は新しい variant に対応する必要がある。両方向を数える一方、0 に置かれていた値が
  0 から離れる動きは数えない。walk の開始状態がまさに
  それになりうる (指令に追従する前の、実現トルクが 0 の reaction wheel)。`Crossing::Either` では
  そこで root を報告し、量が単調な step を分割していた: 100 ms 刻みの run で 0.78 ms の停止を実測し、
  以降の sample が step の格子から外れて 100.78 ms と 200 ms になっていた。0 に到達する動きは
  数えるので、step の終端がちょうど零点に載る場合も取りこぼさない。
- `IntegrationError::RootStillCrossed` を追加した。呼び出し側が state を動かす機会を
  与えられた後もなお越えた側に残っている `RootEvent` を報告する。探索は符号の変化から交差を
  見つけるので、最初から越えた側にある値には残っていない。そこから walk は続けられない。
  続けると、その span の残りをずっと拘束の外で伝播する。
- `RootEvent` を追加。時刻ではなく状態が決める境界を表す。燃焼窓の端は既知の時刻なので
  `Segments` で span を刻めるが、reaction wheel の飽和や推進剤の枯渇は時刻が分からず、
  それを起こすステップの内側で交差を見つける必要がある。event は符号付き関数の零点で、
  数える向き・到達で walk を終えるか・同時に交差した event との順序を自分で申告する。
  `RootSet` が event とそれぞれがステップ間で持つ状態を束ね、`advance_to_roots` が
  `FixedStepper` / `AdaptiveStepper` / `AdaptiveStepper853` のいずれでも境界で止まり、
  その時刻の状態を残す。

  検出は `OdeState::project` の前、生の候補で行う。projection は状態を制約面に戻す操作で、
  交差を示す符号変化を消してしまうことがある。局所化は確定済みの始点から幅を変えて
  再計算する二分探索で、adaptive solver がどちらも dense output を持たないためである。
  探索が試す状態も、root で止まった状態も callback には渡らない。境界の状態は、呼び出し側が
  越えたモードを更新するまで最終ではないので、呼び出し側が stepper から読み、処理を終えてから
  記録する。

  終了判定 `event_check` は `advance_to` と同じもので、聞く場所も同じ (歩き始めの state と、確定した
  通常ステップ)。walk は state が決める境界と呼び出し側が決める条件の両方で止まれる。境界はそのどちらの
  場所でもない。

  set は event の slice と、呼び出し側が所有する `RootSlot` の slice を借りる。そのため event の
  個数は呼び出し側の設定で決まる数でよく、crate は引き続き allocation を行わない。各 event は
  `deactivate` / `activate` で切り替えられる。一方向拘束の「解除」は、拘束していない間は意味を
  持たないためである。切り替えて on にすると guard は再武装し、既に on の event を on にするのは
  何もしない。

  呼び手が守るべき点は DESIGN.md に 2 つ書いた。1 つのステップが含んでよい符号変化は各 event の
  値について 1 回まで。もう 1 つは、探索の間は離散モードを凍結すること。モードの切り替わりを
  跨いで RK4 を再ステップすると、境界までの残り $r$ に対して二分探索は $6r/5$ の刻みを返し、
  到達時刻を $0.2r$ 遅く報告する (刻み 8 通りで実測し、テストで固定した)。
  ([#508](https://github.com/sksat/orts/pull/508), [#509](https://github.com/sksat/orts/pull/509), [#510](https://github.com/sksat/orts/pull/510))
- `Integrator::stepper` を追加。状態とその時刻を保持し、目標時刻を次々に与えて進める
  `FixedStepper` を返す。`stepper` / `from_checked_state` / `advance_to` という 3 つの呼び出しは
  adaptive solver が既に持っていたもので、どこで止まるかを進みながら決める伝播ループは、
  設定された solver が何であれ 1 つの形で書けるようになった。`integrate` / `try_integrate` /
  `integrate_with_events` は、この stepper を `t_end` まで 1 回で進めたものである。([#458](https://github.com/sksat/orts/pull/458))
- `Segments` を追加。system が報告する切り替わりで span を区切った segment を返す。各 item は
  束縛済みの system・区間・先に別の segment があったかを持つ。`Segments::new` はどのループも
  歩けない span を拒否する — `t < t_end` を見てから踏むループは、その判断を solver に
  任せられない。([#458](https://github.com/sksat/orts/pull/458))
- `AdaptiveStepper::from_checked_state` と `AdaptiveStepper853::from_checked_state` を追加。
  `advance_to` は開始状態について event predicate に問い合わせる (level-triggered な event は
  そこで既に成立しうる) が、前の segment が終えた場所から続く segment では不要で、同じ
  `(t, state)` を 2 回問い合わせることになる。([#453](https://github.com/sksat/orts/pull/453))
- `SegmentContext` / `SegmentSystem` / `DynamicalSystem::derivatives_in_segment`
  (既定は `derivatives` へ転送) を追加。既知の不連続時刻で区切られた区間について右辺を
  評価する。切り替わりで step を終えるだけでは足りない: solver の最後の stage が step の
  終端に乗るので、`[a, b)` で on の項が `b` で off と読まれ、RK4 では第 4 stage の重み 1/6 が
  落ちる。`SegmentSystem` で包むと segment が 1 つ束縛され、crate 内の stage ループはどれも
  変更せずに済む。segment ごとに包み直すと、DP45 が accept したステップの `k7` を次のステップの
  `k1` に持ち越すぶんが切り替わりを跨がない。DOP853 は accept 後に `k1` を空にする (reject の
  再試行のときだけ持つ) ので、そちらで落ちるのは調整済みの step size である。([#453](https://github.com/sksat/orts/pull/453))
- `IntegrationError` が `core::error::Error` を実装 (手書き、`thiserror` 不使用、
  `no_std` でも動作)。`?` 連鎖や `Box<dyn Error>` に乗るようになった。([#147](https://github.com/sksat/orts/pull/147))
- テスト: 8 つの積分ループすべてに contract test を追加 (`Integrator` の既定
  実装・Verlet・Yoshida がそれぞれ event 検査なし/ありの 2 本、adaptive stepper
  2 つ)。各ループについて、状態を通知すること、通知時刻が増加すること、直前の
  状態ではなく計算した状態を通知すること、返り値がその状態と一致すること、
  最後の通知が `t_end` に着地すること、導関数が `t` に依存する系で解析解に
  一致することを検査する。固定刻みの 6 ループには、`dt` 刻みで進み最終ステップ
  だけを短縮することも課す。実測: 5 つのループを「初期状態を返して何も通知
  しない」に置き換えると 167 件のうち 147 件が通り、各状態を 1 ステップ遅れて
  通知する欠陥はどのテストも検出しなかった。調和振動子の一周期テストはどちらも
  検出できない (一周期後の解が初期状態と同じ)。`test_systems` の共有試験系は
  すべて時刻引数を無視している。([#408](https://github.com/sksat/orts/pull/408))
- テスト: 保存量のテストが、軌道が実際に動いたことを要求するようになった。
  エネルギー drift も Lotka-Volterra の不変量も、状態が変化しなければ完全に
  保存される。symplectic のテストは長時間積分の後半 drift を前半と比べていた
  ため、初手で全エネルギーを失っても「有界」と読めていた。実測: accept した
  状態を毎ステップゼロにすると 6 件が通り、`Rk4::step` を凍結すると RK4 の
  保存則テスト 2 件が通った。symplectic の 4 件には初期エネルギーで正規化した
  drift の上限を、RK4 の 2 件には状態が移動したことの検査を、Lotka-Volterra
  には `ln` が読む個体数の正値性を加えた。
  `tight_tolerance_dop853_fewer_evaluations` も、評価回数を比べる前に両者が
  `t_end` まで完走して正しい解に達したことを要求する。
  ([#409](https://github.com/sksat/orts/pull/409))

#### Changed
- `Integrator` は `step_unprojected` を要求し、`step` はそれを projection する既定実装になった。
  root event の探索は生の候補を読むが、どの実装も projection の前にそれを作っていた。
  この workspace の外に `Integrator` の実装がある場合は**破壊的変更**で、`step` を
  `step_unprojected` に改名して `project` の呼び出しを外す。([#508](https://github.com/sksat/orts/pull/508))
- stepper の outcome 型を 1 つに統合。`AdvanceOutcome853` は同じ 2 variant のもう 1 つの写しで、
  実行時に solver を選ぶ呼び出し側は同じ 2 つの腕を 2 回書く必要があった。**`AdvanceOutcome853`
  は削除**し、DOP853 の `advance_to` は `AdvanceOutcome` を返す。([#458](https://github.com/sksat/orts/pull/458))
- 固定刻みの積分すべて (`Integrator`・`StormerVerlet`・Yoshida 各型) が、時計に `dt` を
  足し込むのではなく span の開始から数えたグリッドを歩き、最後のステップで span の端を
  代入するようになった。累積はずれ、そのずれは walk が長いほど大きくなる: 0 から `0.1` を
  9 回足すと `0.8999999999999999` になり、残りが `0.10000000000000009` — `dt` をわずかに
  超える — なので `[0, 1]` は 10 ではなく 11 ステップかかり、最後の 1 つは 1 ulp 幅だった。
  **各ステップの時刻が変わる** (walk の初めでは 1 ulp、先へ行くほど大きく — `0.1` を 1000 回
  足した値は開始点基準の `100.0` と複数 ulp 違う)。累積した残りが `dt` を超えていた span は
  ステップ数が 1 つ減る。
- 固定刻みの積分が、step の開始時刻における f64 の間隔より狭い `dt` を拒否するようになった
  (新しい `IntegrationError::StepBelowSpacing`)。従来のループが失敗するのは clock が全く
  動かない場合 (`t + dt == t`) だけで、間隔の半分から 1 つ分のあいだでは要求より遠くまで
  動いていた — `t = 1e15` で `dt = 0.1` は clock を 0.125 進める一方 solver は 0.1 を積分するので、
  返る state は clock が既に通り過ぎた時刻のものだった。**そうした span は、グリッドが運べる
  ぶんを歩いたうえで、その step でエラーになる**。([#458](https://github.com/sksat/orts/pull/458))
- 固定刻みの積分が、着地予定の時刻まで clock を運べない step を拒否するようになった
  (新しい `IntegrationError::LandingUnreachable`)。span の最後の step は端を計算で踏むのではなく
  代入するので、その幅は開始と終端の両方を解像する必要がある。終端が要求する解像度に対して
  `|t|` が十分大きいと、1 つの f64 では両方を解像できない。`-1e16` から `1.0` までの距離は
  `1e16` に丸まり、`-1e16 + 1e16` は `0.0` なので、従来のループは 0 で終わる step を solver に
  渡したうえで、その state を `1.0` のものとして報告していた。408 個の span で実測したところ、
  この拒否が届くのは、`1e14` 以上離れた場所から 1 ステップで 0 を跨ぐ span だけである。([#458](https://github.com/sksat/orts/pull/458))

#### Fixed
- `RootSet::scan_step` が、二分探索で試した各幅の値を読み、step の両端では符号差がなかった
  event がそこで bracket されていれば、step の始点から探索し直すようになった。値が尽きて戻る
  までが 1 step に収まる event は両端が同符号なので、従来は見送られていた。その負の区間に入る
  幅を試した時点で符号差が現れ、step 内で最も早い交差が報告される。探索が止まる幅だけを読むのでは
  足りない: 探索は零点の後側で止まるため、その bracket の内側に 2 つ目の零点があると値は始点と同じ
  側に戻っている (許容 $10^{-3}$ では $0.4 - t$ の零点は $0.400390625$ で求まり、
  $(t-0.2)(t-0.4001)$ はそこでは正である)。候補は 1 回の探索のあいだ入れ替えずに増やす:
  $[0, 1]$ で $a = 0.8 - t$、$b = (t-0.2)(t-0.9)$、$c = (t-0.1)(t-0.3)$ のとき、$b$ を
  0.2 で求めた時点で $a$ は交差しなくなるため、候補の件数が増えなくなったら止める方式では
  0.2 を commit して $c$ の 0.1 の交差を落とす。各パスには `max_iterations` の回数が
  そのまま与えられ、求めた時刻で新たに bracket される event がなければ再積分は発生しない。
  2 つの零点のあいだに何もない場合の呼び出し側の義務 (1 step につき各 event の符号変化は
  1 回まで) は変わらない。
- 積分ループが、渡された状態そのものについて event の判定を行うようになった
  (1 歩進める前)。`utsuroi` の 5 経路と、`IndependentGroup` / `CoupledGroup` が
  自前で持つ固定刻みループが対象。「地表より下」「この高度より下」のような level-triggered な
  event は `t0` で既に成立しうるが、従来はまず 1 歩進めていたため、判定関数自身が
  無効と呼ぶ状態で 1 step 遅れて報告していた。`orts run --sat altitude=50
  --duration 100` (地球の大気境界は 100 km の Kármán line) は
  「atmospheric entry at 50.0 km」を t = 10 s で報告し、その時刻の sample も
  記録していた。報告している entry から 78 km 進んだ地点である。現在は t = 0 で
  開始状態のまま停止する。([#419](https://github.com/sksat/orts/pull/419))
- `Integrator::try_integrate` が、結果が非有限になった最初のステップで
  `IntegrationError::NonFiniteState` を返して停止するようになった
  (`integrate_with_events` が既に行っていた検査)。従来は `NaN` 状態のまま
  span 全体を回して `Ok` を返していたため、`orts serve` の制御ループが
  その状態からセンサを読み、plugin controller に渡し、成功を報告していた。([#335](https://github.com/sksat/orts/pull/335))

### `tobari` (Rust, crates.io)

#### Added
- `gravity::SphericalHarmonicCoefficients`: 静的 ICGEM `.gfc` parser (fully normalized
  な `gfc` record のみ。時変 record `gfct`/`trnd`/`dot`/`asin`/`acos` と
  `unnormalized` は明示エラーで拒否。`errors` の列数、`m ≤ n`、重複、非有限値、
  `C00 = 1`、degree-1 がゼロであること、係数の完備性を検証。`norm` は必須、
  header key の二重宣言はエラー、`max_degree` は `gravity::MAX_DEGREE` = 2190
  が上限)。`from_icgem(text, max_degree)`, `from_icgem_reader(BufRead,
  max_degree)`, `from_icgem_file(path, max_degree)` は任意の degree 上限を取り、
  その三角が揃った行で読み止めるので、degree 2190 の EGM2008 file に 70×70 を
  要求しても 70×70 分しか確保しない。file が持たない degree の要求は
  `IcgemParseError::DegreeUnavailable`。accessor は `gm()`, `radius()`,
  `tide_system()` (記録のみ、変換しない), `coefficient(n, m)`) と、
  `gravity::SphericalHarmonicField`: 一つの係数集合 (`Arc` で共有) の
  degree × order 窓に対する body frame での非中心 potential / 加速度の
  Holmes–Featherstone 評価器 (km 単位)。`new(coefficients, degree, order)` /
  `full` / `truncated` は係数集合が提供できない窓に対して clamp せず
  `TruncationError` を返す。Orekit の `HolmesFeatherstoneAttractionModel` と
  同じ構造だが極そのものでも正則。70×70 まで Orekit と点ごとに 1e-13·GM/r² で一致
  (`tests/oracle_geopotential.rs`、fixture は
  `tools/generate_orekit_geopotential_fixtures.py`)。
  ([#411](https://github.com/sksat/orts/issues/411))
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
- recording が伝播された frame を読み (CSV の `# frame`、`.rrd` の `meta/sim/frame`)、
  `simple-eci` 以外の frame の recording は描かずにメッセージ付きで reject する。
  viewer はすべての state に SimpleEci (ERA のみ) の地球回転を掛けるので、`gcrs` の
  recording は地球固定位置と ground track が黙って誤る。frame の無い recording は
  このフィールドより前のもので `simple-eci`。([#411](https://github.com/sksat/orts/issues/411))
- 外乱トルクを model ごとに 1 チャートで表示し、body frame の 3 成分を重ねる。
  外乱トルクで誤るのは向きで、magnitude では反対向きに回している場合と区別が付かない。
  そのため norm ではなく x, y, z を別系列として描く。チャートは実行が持つ model
  (`gravity_gradient`, `panel_srp`, `panel_drag`) ごとに現れ、複数衛星では各衛星の 3 軸に
  その衛星名を付ける。1 機だけ見るときは legend で系列を isolate する。実行が持たない
  model の列は 0 でなく空にし、チャートは「トルクが 0 と測れた」ではなく欠測として描く。
  ([#471](https://github.com/sksat/orts/pull/471))
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
- **破壊的変更:** `SatelliteState` が、衛星が姿勢について述べる 3 つの事実のどれかを言えるように
  した。従来の `attitude` には「姿勢を名乗ったが回転として使えない」を表す方法が無かったため、
  その事実を持つ呼び出し側は、viewer が有効でないと判定する値 (NaN や全 0) を渡すしかなく、
  どの値がそうなるかは公開型に現れなかった。quaternion と拒否を型で排他にした:
  `Quat` と `attitudeRefused: true` の同時指定は型エラーで、`attitude: undefined` は
  どちら側にも置ける。`attitude` だけを渡す形 (`Quat | undefined` を含む) は従来どおり通る。
  scene は境界で状態を解決し、位置と並べて運ぶので、拒否を `OrbitPoint` に符号化しない。
  ([#479](https://github.com/sksat/orts/pull/479))
- サンプルが姿勢について述べる内容を 1 箇所で解決するようにした。`resolveAttitude` が
  `absent` / `refused` / `usable` を返し、消費側 6 箇所が組み合わせていた 2 つの述語を置き換える。
  #451 が挙げる 10 件の欠陥のうち 6 件はこの構造から出ていた (回転・マーカーの形・登録モデルを
  描くか・拡大率の各帰結がそれぞれ状態を再導出していた)。返る答えが 3 つ変わる。いずれも
  定義域の端で、2 つの述語が表示側の実際の扱いと食い違っていた場所である:
  - ノルム自体が overflow する大きさで書かれた quaternion (`Math.hypot` が Infinity になる
    `[MAX_VALUE, MAX_VALUE, 0, 0]`) を正規化して使うようにした。従来は拒否していた。
    これは `[1, 1, 0, 0]` と同じ回転を指す。
  - claim が不完全なサンプル (`qw` があって `qx`/`qy`/`qz` が無い) を挟む補間の結果が
    `absent` でなく `refused` になる。拒否が再生中も保たれ、viewer が使わないと判断した
    claim から登録済みの宇宙機モデルが描かれることがなくなる。
  - サンプル自身の時刻を問われたとき、次のサンプルが姿勢を持たなくても自身の回転を返すように
    した。`TrailBuffer.interpolateAt` はこの場合を fraction 0 として呼ぶので、記録が持っている
    回転が落ちていた。
  ([#478](https://github.com/sksat/orts/pull/478))
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
- CSV を開いたとき、ファイルが持つ header に従って列を読むようにした。これでモデルごとの
  トルクと姿勢が viewer に届く。従来は列を位置で読み `nu` で止めていて、`orts run` は
  角速度・トルク・quaternion をその後ろに書くため、すべて落としていた。空セル（列は fleet
  全体の和集合なので、そのモデルを持たない衛星の欄は空になる）は値を入れないままにする。
  `Number("")` は 0 になり、「トルクが 0 と測れた」と読めてしまうため。header を持たない
  ファイルは従来どおり位置で読む。400 km で `diag(10, 40, 45)` を傾けた CSV で実測:
  gravity-gradient トルク 6.720e-5 N·m で、`.rrd` と live wire と同じ値。
  ([#477](https://github.com/sksat/orts/pull/477))
- モデルごとのトルクを持つ `.rrd` を開いたときに、そのトルクチャートが出るようにした。
  値は記録に入っている (`orts run` が書く) が decoder が落としていたため、live 実行では
  チャートが出るのに同じ実行をファイルから再生すると出なかった。モデルが「ある」と見なすのは
  その衛星について完全な triple が decode されたときで、fleet の記録（列は衛星全体の和集合）で
  モデルを持たない衛星に出ることはない。400 km で `diag(10, 40, 45)` を傾けた記録で実測: チャートは
  gravity-gradient トルク 6.720e-5 N·m を持ち、同じ構成を live wire で測った値と一致する。
  ([#476](https://github.com/sksat/orts/pull/476))
- chunk 単位のファイル読み込みが終わった直後に届いたサンプルが二重に数えられていた。
  読み込みは各衛星の ingest buffer に trail の配列そのものを置換データとして渡すが、
  buffer はその参照を保持し、以後の push は別に queue され、両者を連結して返す。
  そのため間に届いた点が両方に現れていた。snapshot を渡すようにした。
  ([#474](https://github.com/sksat/orts/pull/474))
- 実行中に追加した衛星のチャートが出るようにした。`satellite_added` は WebSocket
  hook までは届いていたが、そこで止まっていたため、チャートの表示条件を作る Info
  snapshot に追加した衛星が入らず、その衛星の加速度もトルクもチャートが出なかった。
  ([#474](https://github.com/sksat/orts/pull/474))
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
- `TimeSeriesChart` に `spanGaps` を追加（既定 `true`、従来の挙動）。multi-series の
  データは、ある系列に sample が無く別の系列にはある時刻で gap を持つので、そこを線で
  つなぐのが正しい。gap が「その時刻に値が存在しない」を意味する呼び出し側（実行が持たない
  モデルの列など）は `false` を渡し、線を切って、計算されていない値を描かないようにする。
  ([#471](https://github.com/sksat/orts/pull/471))
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

#### Added
- decoder がモデルごとの body torque (`<model>.torque_body_{x,y,z}_Nm`) を、
  viewer がチャートにするモデルについて 1 モデル 1 triple で返すようにした。triple の一部しか
  記録されていない場合は、姿勢が不完全なときと同じく報告しない（欠けた成分は作れない）。
  トルクの列は曖昧行の判定にも入れたので、ある時刻に 1 軸の値が 2 つある記録で列をまたいで
  組み合わせることはない。
  ([#476](https://github.com/sksat/orts/pull/476))

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

