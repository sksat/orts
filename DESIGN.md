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

viewer は **Source** を primitive とするデータ駆動アーキテクチャを採用する:
- **WebSocket source**: `orts serve` や `orts replay` に接続し、シミュレーション結果をストリーミング受信
- **ローカルファイル source**: CSV / RRD ファイルを Web Worker でパースし、チャンク単位で投入

viewer にはモード切替の概念はなく、全ての source が同一の TrailBuffer + IngestBuffer パイプラインにデータを流す。SourceAdapter が入力元の差異を吸収し、SourceEvent (discriminated union) を通じて統一的にデータを配信する。将来的には複数 source の同時接続・比較表示にも対応する。

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
orts run [OPTIONS]                           # シミュレーション実行 → .rrd 保存（デフォルト）
orts run --output stdout --format csv        # CSV を stdout に出力
orts serve [OPTIONS]                         # WebSocket サーバー
orts convert <input> --format csv            # フォーマット変換（rrd → csv）
```

### run オプション
- `--altitude <km>` (default: 400) — 軌道高度
- `--body <name>` (default: earth) — 中心天体
- `--dt <seconds>` (default: 10) — 積分時間刻み
- `--output-interval <seconds>` — 出力間隔（dt と独立）
- `--output <path|stdout>` (default: output.rrd) — 出力先
- `--format <rrd|csv>` (default: rrd) — 出力フォーマット

### serve
WebSocket サーバーモード。viewer へシミュレーション結果をリアルタイムストリーミング。

`SimGroup` enum で `OrbitalSystem`（軌道のみ）と `SpacecraftDynamics`（姿勢-軌道結合）を動的切替:
- attitude config（慣性テンソル・質量）なし → `OrbitalSystem`（従来動作）
- attitude config あり → `SpacecraftDynamics` + 重力傾斜トルク（デフォルト ON）

姿勢データは `AttitudePayload`（body-to-inertial quaternion [w,x,y,z] + angular_velocity + source）として
`WsMessage::State` / `HistoryState` に optional で含まれ、全経路（live/history/query_range）で保持される。

### convert
フォーマット変換。rrd → csv などをサポート。

## Crate 構成

Rust crate と TypeScript パッケージで構成される。
Rust crate は `orts-` prefix を使用し、ディレクトリ名は prefix なし。
汎用的な独立ライブラリには固有名を付ける（kaname, uneri, tobari）。

### Rust crate

| Crate | ディレクトリ | 責務 |
|---|---|---|
| utsuroi | `utsuroi/` | 汎用 ODE ソルバ。OdeState trait、DynamicalSystem trait、RK4、Dormand-Prince 適応刻み、Yoshida シンプレクティック積分 |
| kaname | `kaname/` | 測地学・天文ライブラリ。詳細は [`kaname/DESIGN.md`](kaname/DESIGN.md) |
| tobari | `tobari/` | 地球周辺環境モデル（大気密度・地磁気・宇宙天気）。詳細は [`tobari/DESIGN.md`](tobari/DESIGN.md) |
| orts | `orts/` | 軌道力学 + シミュレーション。重力場 (J2+)、ケプラー要素、TLE、摂動モデル (`Model<S>`: 抗力/SRP/第三体)、OrbitalSystem、イベント検出、姿勢力学 (AttitudeState)、SpacecraftState (軌道+姿勢+質量)、capability traits (`HasAttitude`/`HasOrbit`/`HasMass`)、ECS データモデル・Rerun エクスポート |
| orts-cli | `cli/` | CLI + WebSocket サーバ。run/serve/convert サブコマンド。バイナリ名は `orts` |

### TypeScript パッケージ

| パッケージ | ディレクトリ | 責務 |
|---|---|---|
| @orts/uneri | `packages/uneri/` | DuckDB-wasm + uPlot 汎用時系列チャートライブラリ |
| viewer | `viewer/` | リアルタイム 3D 軌道ビューア (React + @react-three/fiber) |

### 依存関係

```
kaname              utsuroi               ← 基盤層
  ↑   ↑                  ↑
  │   tobari              │               ← 環境層
  │     ↑                 │
  │     orts ─────────────┘               ← 軌道力学 + シミュレーション層
  │       ↑        ↑ tobari
  └── orts-cli                            ← アプリ層
```

- kaname と utsuroi は独立（ワークスペース内の他クレートに依存しない）
- tobari は kaname のみに依存し、地球周辺環境モデル（大気密度 + 地磁気）ライブラリとして独立性を維持
- orts は integrator, kaname, tobari を利用（軌道力学 + 摂動モデル + 姿勢力学 + 宇宙機統合 + データ記録）
- orts-cli は orts を利用する薄い CLI ラッパー
- utsuroi は汎用 ODE ソルバであり、軌道力学・姿勢力学の両方から利用される

### 設計原則

- **capability-based モデル合成**: state の capability trait (`HasAttitude`, `HasOrbit`, `HasMass`) と統一 `Model<S>` trait により、モデルが必要な state 情報を generic bound で宣言し、対応する system で自動的に使える設計。`GravityField`、`AtmosphereModel`、`MagneticFieldModel` 等の環境 trait は別途維持
- **問題に応じたモデル構成**: ユーザーが OrbitalSystem / SpacecraftDynamics に重力場とモデルを組み合わせる。フルスペック計算は不要な場合がほとんど
- **責務の分離**: 各 crate は独立テスト可能。座標変換は kaname、環境モデル（大気・磁場）は tobari、シミュレーションは orts

## 実装済みのアーキテクチャ拡張

### 姿勢力学 (orts::attitude)

軌道力学 (並進 6 自由度) に加え、姿勢力学 (回転 3 自由度) を扱うモジュール (`orts/src/attitude/`)。

- `AttitudeState`: 四元数 (Hamilton, スカラー先頭) + 角速度 (機体座標系)。7 状態量、OdeState 実装
- `TorqueModel` trait: ForceModel の姿勢版。`torque(t, state, epoch) -> Vector3`
- `GravityGradientTorque`: 重力傾斜トルクモデル。`position_fn` で軌道状態を外部注入可能（非結合モード用）
- `CoupledGravityGradient`: 結合モード用。`HasAttitude + HasOrbit` から位置を直接取得。物理計算は `gravity_gradient_torque_vector()` を共有
- `AttitudeSystem`: DynamicalSystem 実装。Euler の回転方程式 + 四元数運動学

姿勢モジュールは orbits に依存しない（モジュール境界で設計上の分離を維持）。
姿勢のみの伝播は `AttitudeSystem` 単体で可能（mock トルク・mock 軌道の注入に対応）。

### Integrator のジェネリック化 (実装済み)

**OdeState trait**: RK 法が必要とする代数演算を抽象化。

```rust
pub trait OdeState: Clone + Sized {
    fn zero_like(&self) -> Self;
    fn axpy(&self, scale: f64, other: &Self) -> Self;  // self + scale * other
    fn scale(&self, factor: f64) -> Self;
    fn is_finite(&self) -> bool;
    fn error_norm(&self, y_next: &Self, error: &Self, tol: &Tolerances) -> f64;
    fn project(&mut self, _t: f64) {}  // 四元数正規化等 (default no-op)
}
```

**DynamicalSystem trait**: Associated type で状態型を指定。

```rust
pub trait DynamicalSystem {
    type State: OdeState;
    fn derivatives(&self, t: f64, state: &Self::State) -> Self::State;
}
```

RK4、Dormand-Prince は OdeState の trait メソッドのみを使い、具体的な型を知らない。
`State` (軌道6次元) と `AttitudeState` (姿勢7次元) がそれぞれ OdeState を実装する。

### Capability-based Model trait

モデル（摂動力、トルク、スラスタ等）を統一 `Model<S>` trait で表現する。
モデルは state の capability trait を generic bound で宣言し、対応する system で自動的に使える。

旧設計（ForceModel / TorqueModel / LoadModel の3本立て）では、system 間でモデルを載せ替える際にアダプタ（ForceModelAtCoM, TorqueModelOnly）が必要だった。
新設計では capability bound により、同一のモデルが対応する全ての system で直接使える。

#### Capability traits

state 型が持つ情報を表す trait:

```rust
pub trait HasAttitude { fn attitude(&self) -> &AttitudeState; }
pub trait HasOrbit   { fn orbit(&self) -> &OrbitalState; }
pub trait HasMass    { fn mass(&self) -> f64; }  // 並進質量のみ。慣性テンソルは system 側
```

| State 型 | HasOrbit | HasAttitude | HasMass |
|---|---|---|---|
| `OrbitalState` | ○ | - | - |
| `AttitudeState` | - | ○ | - |
| `SpacecraftState` | ○ | ○ | ○ |

`SpacecraftState` は全 capability を実装するため、部分的な state 向けに書かれたモデルが自動的に `SpacecraftDynamics` でも動く。

#### 統一 Model trait

```rust
pub trait Model<S>: Send + Sync {
    fn name(&self) -> &str;
    fn eval(&self, t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads;
}
```

モデルの capability bound と使用可能な system の対応:

| モデル例 | Bound | OrbitalSystem | AttitudeSystem | SpacecraftDynamics |
|---|---|---|---|---|
| AtmosphericDrag | `S: HasOrbit` | ○ | - | ○ |
| PD制御則 | `S: HasAttitude` | - | ○ | ○ |
| Thruster | `S: HasAttitude + HasOrbit + HasMass` | - | - | ○ |

#### Dispatch 方式

system は `Vec<Box<dyn Model<ConcreteState>>>` で異種モデルのコレクションを保持する（動的 dispatch）。

- **コンパイル時に静的解決**: capability bound により「このモデルがこの system で使えるか」はコンパイル時に判定される。アダプタやキャスト不要
- **実行時に動的 dispatch**: 異種モデルの collection 呼び出しは vtable 経由。ODE 内部ループ (axpy/scale) がホットパスであり、モデル評価のオーバーヘッドは無視できる
- 性能がボトルネックになる場合に備え、静的 dispatch パス（generic system 等）を将来追加可能

#### ExternalLoads の不変条件

`ExternalLoads` は全モデルの共通の返り値型。以下の不変条件を持つ:

- `acceleration_inertial`: **慣性座標系** [km/s²]、加算的合成
- `torque_body`: **機体座標系** [N·m]、加算的合成
- `mass_rate`: [kg/s]、加算的合成（負=消費）
- 全モデルは同一の immutable state snapshot に対して評価される（評価順序に依存しない）
- **Model は純関数的評価**: `eval(&self, ...)` は副作用を持たない

OrbitalSystem は `torque_body` を無視し、AttitudeSystem は `acceleration_inertial` を無視する。

#### re-export 方針

`orts::model` を正規のインポートパスとする。`Model`, `HasAttitude`, `HasOrbit`, `HasMass`, `ExternalLoads` を公開。

## 将来のアーキテクチャ拡張

### orts: 宇宙機型・シミュレーション管理・データ記録

姿勢-軌道結合、複数衛星、衛星分離に対応するためのクレート。
spacecraft 型は結合積分の前から有用（衛星の「位置+姿勢」の統一表現として）なので最初から含める。
独立した orts-spacecraft クレートも検討したが、クレート数の増加を避け、将来必要になったら切り出す方針とした。

#### モジュール構成

```
orts (orts/)
  src/
    model.rs          # Model<S> trait, ExternalLoads, HasAttitude/HasOrbit/HasMass capability traits
    perturbations/    # AtmosphericDrag, SRP, ThirdBodyGravity (impl<S: HasOrbit> Model<S>)
    orbital/          # OrbitalState, OrbitalSystem, GravityField, Kepler, TwoBody
    attitude/         # AttitudeState, AttitudeSystem, GravityGradientTorque
    spacecraft/       # SpacecraftState, SpacecraftDynamics
    group/            # PropGroup, GroupState, InterSatelliteForce
    record/           # Recording, Rerun export (default feature)
    setup.rs          # build_orbital_system(), build_spacecraft_dynamics() ヘルパー
    lib.rs
```

#### spacecraft モジュール — 宇宙機の状態と結合力学

**SpacecraftState**: 軌道(6) + 姿勢(7) + 質量(1)。OdeState 実装。
全ての capability trait (`HasOrbit`, `HasAttitude`, `HasMass`) を実装し、
`Model<S>` の capability bound を満たすことで全モデルの統合先となる。

**SpacecraftDynamics**: `DynamicalSystem for SpacecraftState`。
`GravityField` + 慣性テンソル + `Vec<Box<dyn Model<SpacecraftState>>>` から合成。

DynamicalSystem 実装の3層は排他的選択:

| 層 | 型 | 状態 | モデル格納 |
|---|---|---|---|
| 軌道のみ | `OrbitalSystem` | `OrbitalState` (6D) | `Vec<Box<dyn Model<OrbitalState>>>` |
| 姿勢のみ | `AttitudeSystem` | `AttitudeState` (7D) | `Vec<Box<dyn Model<AttitudeState>>>` |
| 結合 | `SpacecraftDynamics` | `SpacecraftState` (14D) | `Vec<Box<dyn Model<SpacecraftState>>>` |

`impl<S: HasAttitude> Model<S>` で実装されたモデルは AttitudeSystem と SpacecraftDynamics の両方で使える。
`impl<S: HasOrbit> Model<S>` で実装されたモデルは OrbitalSystem と SpacecraftDynamics の両方で使える。
アダプタは不要。

**実装済みの姿勢依存モデル**:
- PanelDrag（投影断面積 + 圧力中心オフセット）— `impl<S: HasAttitude + HasOrbit + HasMass> Model<S>`
- PanelSrp（太陽パネル向き）— 同上
- Thruster（推力方向 + 燃料消費 + ThrustProfile による制御分離）— 同上

#### group モジュール — 複数衛星のグループ管理

**PropGroup trait**: 型消去されたグループ制御、内部は monomorphic。

```rust
trait PropGroup {
    fn epoch(&self) -> Epoch;
    fn ids(&self) -> &[SatId];
    fn propagate_to(&mut self, t: Epoch) -> Result<()>;
    fn snapshot(&self) -> GroupSnapshot;
}
```

**GroupState<S: OdeState>**: `Vec<S>` ベースの可変N機結合状態。
- error_norm: 衛星ごとの正規化誤差の max（設定可能: max/RMS/グループ分け）
- N増加（衛星分離イベント）にも対応可能

**異種グループの相互作用**: `HasPosition`, `HasEpoch`, `HasId` trait でグループ間イベント検出。

**Split/Merge**: 停止 → 状態再構成 → 新システムで再開（IntegrationOutcome::Terminated パターン）。

#### scheduler モジュール — 積分レジームと同期

3つの積分レジーム:
- **独立**: 各グループが最適 dt で積分（弱結合、遠距離）
- **同期**: 各自の dt + 共通同期点で状態交換（中結合、制御力交換）
- **結合**: 単一 ODE で1つの dt（強結合: テザー、近接）

レジーム遷移の安定性:
- 距離閾値にヒステリシス（結合開始閾値 < 分離閾値）
- 最小滞留時間（チャタリング防止）
- 分離時の不連続処理（インパルス、質量/慣性ジャンプ）→ 積分器リスタート

#### record モジュール — データ記録

現 orts-datamodel の内容を統合:
- Recording, Component, Archetype trait
- Rerun (.rrd) エクスポート（default feature `rerun`; `default-features = false` で除外可能）
- Recorder trait（CLI/WS からの記録インターフェース）

#### 状態ベクトル 3層設計

| 層 | 型 | 次元 | 用途 |
|---|---|---|---|
| Layer 1 | `State`, `AttitudeState`, `SpacecraftState` | 6, 7, 13+ | 固定次元、monomorphized、パフォーマンスクリティカル |
| Layer 2 | `GroupState<S: OdeState>` | 可変 | Vec<S> ベース、runtime で N 変更可、error_norm は衛星ごと max |
| Layer 3 | レジーム遷移 | — | イベント駆動で停止→再構成→再開。ヒステリシス+最小滞留時間 |

DynamicalSystem::State の associated type は常に静的に決定（monomorphization）。
Vec\<f64\> ベースの動的状態も検討したが、ODE 内部ループ (axpy/scale) はホットパスであり monomorphization が重要。
可変 N は GroupState\<S\> の Vec\<S\> で対応し、各衛星の内部演算は固定次元のまま。
CLI でのランタイム選択は `enum Sim { Orbit(...), Spacecraft(...) }` で分岐し、内部は monomorphic。

#### 実施フェーズ

| フェーズ | 内容 |
|---------|------|
| Phase A | spacecraft モジュール（SpacecraftState）+ record モジュール **実装済み** |
| Phase B | 姿勢-軌道結合: SpacecraftDynamics, 姿勢依存 drag/SRP **実装済み** |
| Phase B' | Capability-based `Model<S>` trait 移行 **実装済み** |
| Phase C | 複数衛星: group + scheduler モジュール、CLI 簡素化 |
| Phase D | 姿勢制御検証基盤 **実装中** |
| Phase E | 姿勢 viewer 接続: serve→WS→viewer パイプラインに姿勢データ追加、SimGroup enum |
| Phase P | プラグイン可能な制御則・ミッションロジック（WASM guest で制御則を再コンパイルなしに差し替え） |

#### Phase D 詳細: 姿勢制御シミュレーション 3層アーキテクチャ

Basilisk/Orekit と同様の3層分離を採用:

| 層 | trait | 状態 | 用途 |
|---|---|---|---|
| ContinuousModel | `Model<S>` (既存) | なし（純関数） | drag, SRP, gravity gradient, memoryless 制御則 |
| StateEffector | `StateEffector<S>` | ODE 状態の一部 | RW 角運動量、ジンバル角（力学バックリアクション） |
| DiscreteController | `DiscreteController` | 内部状態（`&mut self`） | PID, B-dot(有限差分), カルマンフィルタ, モード遷移 |

**ContinuousModel の境界**: memoryless 制御則のみ。サンプル信号、フィルタ、anti-windup、モードロジックは DiscreteController に属する。

**StateEffector**: `AugmentedState<S>` で ODE 状態ベクトルを外側から拡張。`SpacecraftState` は変更なし。
名前付きサブステート（`AuxRegistry`）で raw indexing を回避。

**DiscreteController**: 固定サンプル周期で segment-by-segment 積分。制御区間内はコマンド凍結（adaptive solver の内部サブステップでも不変）。共有可変状態（`Arc<Mutex>`）は使わない。

**Phase D-0 実装済み**: DecoupledAttitudeSystem, AttitudeReference, PD制御則
**Phase D-1 実装済み**: B-dot デタンブリング（stateless 解析近似）+ 地磁気モデル（TiltedDipole + IGRF-14）
**Phase D-2 実装済み**: DiscreteController 基盤 + B-dot 有限差分版
**Phase D-3 実装済み**: StateEffector + AugmentedState + ReactionWheel
**Phase D-4 実装済み**: 統合テスト（PID + RW + 環境トルク）
**Phase D-5**: MagneticFieldModel trait 抽象化 + ジェネリクス化（BdotDetumbler\<F\> 等）+ IGRF 球面調和展開

#### Phase P 詳細: プラグイン可能な制御則・ミッションロジック

宇宙機の姿勢制御則・スラスタ噴射スケジュール・ミッションモードマシンを、host (orts) から分離して外部記述のプラグインとして差し替え可能にする。再コンパイルなしに制御則を試行錯誤できるようにすることが目的。

##### 設計方針

**P-D1. プラグインは サンプル tick でのみ呼ぶ (ホットパスから追い出す)**

guest は ODE RHS のホットパスから完全に外に出す。サンプル tick (segment 境界) でのみ呼び出し、返ってきた command は Rust 側 actuator に ZOH でセットして次の区間を native Rust で積分する。既存の `DiscreteController` の segment-by-segment パターンにそのまま乗る。

射程は **制御側のみ**。環境モデル (drag, SRP, magnetic field 等、ODE RHS で評価されるもの) のプラグイン化は Phase P の対象外。event-triggered guest 呼び出し (閾値超え、anomaly detection) は utsuroi の event detection + segment 再起動 (`IntegrationOutcome::Terminated` パターン) で対応する。

**P-D2. 戻り値は物理量ではなく Command (論理指令)**

`ExternalLoads` (acceleration_inertial, torque_body, mass_rate) を guest に直接返させない。代わりに「magnetic moment [A·m²]」「RW command」「throttle 0..1」「impulsive Δv」といった論理的コマンドを返させる。物理モデルと制御則の分離を保ち、Rust 側 actuator (`CommandedMagnetorquer`, `DynamicThrottle`+`Thruster`, `ReactionWheelAssembly`) が物理化を担当する。

Command enum は最小 variant から始めて phase ごとに拡張する (early lock-in 回避):
- P1 (Detumbling): `Command::MagneticMoment(Vector3<f64>)` 1 variant のみ
- P3 (PD + モードマシン): `Command::RwCommand(RwCommand)` を追加
- P4 (推進): `Command::Throttle` / `Command::ImpulsiveDv` を追加
- P5 (結合): 複数 variant 組み合わせ

**P-D3. trait 構造: 既存 `DiscreteController` を拡張して 1 trait に統一**

独立 `ControllerBackend` trait は作らない。`DiscreteController` を以下のように拡張する:

- `type Command` を共通 `enum Command` に差し替え
- `(attitude, orbit, epoch)` を `Observation` struct にまとめる (env snapshot 追加に備える)
- `name()` / `api_version() -> u32` / `init(config) -> Result` / `current_mode() -> Option<&str>` / `snapshot_state()` / `restore_state()` を default impl 付きで追加
- `NativeController` / `WasmController` 等が `impl DiscreteController` を直接提供、adapter 不要
- trait bound は `Send` のみ (`Sync` は不要、wasmtime `Store: !Sync` と整合)

**P-D4. 環境情報は tick 開始時の immutable snapshot を一括渡し**

env snapshot (magnetic field B, sun direction, atmospheric density, current epoch, ...) は、tick 開始時に host が必要な値を全部計算して 1 つの immutable struct として guest に渡す。tick ごとの host function 個別呼び出しは避ける (決定論性、言語非依存性、marshalling コストで有利)。marshalling は `postcard + serde` を採用 (tick あたり ~500 ns overhead、`api_version: u32` を先頭フィールドに固定)。

**P-D5. 既存 Rust 実装は維持、プラグインは追加バックエンド**

`BdotFiniteDiff`, `InertialPdController` 等は削除しない。Oracle テストで「Rust 実装 vs 同じロジックのプラグイン」の同等性検証に流用する。決定論性の基準:

- Rust native と WASM backend は **1e-12〜1e-14 の bit 近似一致**を目指す
- backend 追加時は Rust 実装との同等性テストを pass しないとマージしない (CI enforce)

**P-D6. ミッションモードマシンは 1 プラグイン内部で分岐**

モード切替 (Detumble → Nadir → Burn) は host が guest を差し替えるのではなく、guest 1 つの内部で分岐する。guest が `current_mode()` で現在モードを observability として公開し、host/viewer がそれを拾う。

##### 第一 backend: WASM (wasmtime + Pulley)

`DiscreteController` trait には複数の backend 実装を接続できる (NativeController / 将来の Rhai / PyO3 等)。第一候補として採用するのは **WASM backend**。以下はその WASM backend の内部設計。

###### インターフェース層: Component Model + WIT

orts と宇宙機プラグインの「契約」を定義する層。`SpacecraftState` record、`Command` enum、`Observation` struct、env import などを `.wit` に宣言的に記述し、`wit-bindgen` が Rust / C / JS / Python / Go の guest bindings を自動生成する。**orts の長期的な API 契約として安定化させる対象**はここ。

- core wasm (i32/i64/f32/f64) の上に record / variant / list / string / resource 等の高級型を乗せる上位レイヤ
- wit-bindgen は 0.X 系で semver 破壊的変更頻発 → `=x.y.z` で厳格 pin 必須
- `jco` で `.component.wasm` → ES module に transpile 可能。viewer (ブラウザ) で同じ guest を動かす余地を残す

Phase P1 では WIT に記述したインターフェースを固定し、guest を常に Component として扱う (wasmi を採用しない理由もここ — wasmi は Component Model 非対応)。

###### 配布フォーマット: `.wasm` / `.cwasm`

同じ component を、portable な標準バイナリ (`.wasm`) として配布するか、wasmtime 固有の事前コンパイル済みアーティファクト (`.cwasm`) として配布するかを選べる。**インターフェース契約には影響せず**、「起動時間 / 配布バイナリサイズ / portability」のトレードオフ。同じ 1 つの component から `cargo component` で `.wasm` を、`Engine::precompile_component` で `.cwasm` を両方生成できる。

| 配布形式 | 生成 | 読み込み | 特徴 |
|---|---|---|---|
| `.wasm` (標準バイナリ) | `cargo component build` / `wasm-tools component new` | `Component::new(&engine, bytes)` で実行時コンパイル | portable。どの wasm ランタイムでも読めるが、実行時に compile 層 (Cranelift) が必要 |
| `.cwasm` (wasmtime 固有) | `Engine::precompile_component(&component_bytes)` | `unsafe { Component::deserialize_file(&engine, path) }` | deserialize のみで即起動。ランタイムに compile 層が不要。wasmtime バージョン一致が必須 |

**Pulley target の `.cwasm` は Pulley bytecode をシリアライズしているだけ** なので ISA-independent。ビルドマシンと実行マシンの CPU が違っても動く (wasmtime バージョン一致は必須)。

SAFETY: `Component::deserialize_*` は `unsafe`。untrusted な `.cwasm` ロードは任意コード実行リスク、信頼できるビルドパイプライン前提。

想定する運用パターン:
- 開発中: `.wasm` を直接差し替え、compile 層込みの orts-cli で実行
- production 配布: CI で `.cwasm` を事前生成、end-user には軽量 orts-cli + `.cwasm` を配る
- 第三者プラグイン: `.wasm` 配布 (wasmtime バージョン依存を避ける)

補足: wasmtime の API では `Module::*` (core wasm) と `Component::*` (Component Model) が並行分離している。Phase P1 では guest を常に Component として扱うので、本設計では `Component::*` 系のみ使う。

###### 実装基盤: wasmtime + Pulley

WASM ランタイム調査 (wasmtime / wasmer / wasmi / stitch / wasm3 / extism / Cranelift 直接) + Phase P0 smoke test の結果、wasmtime を採用する。

wasmtime 内部の役割分担:
- **Cranelift**: wasm → Pulley bytecode への compile 層。`Component::new` / `Engine::precompile_component` が内部で利用
- **Pulley**: pure Rust portable interpreter (wasmtime 内蔵)。`Config::target("pulley64")` で compile target として選択すると、Cranelift は機械語ではなく Pulley bytecode を emit し、実行は Pulley interpreter が行う
- 注: Pulley は wasmtime の `Strategy` enum には存在せず、target triple で選ぶ方式

採用理由:
- Component Model / wit-bindgen を first-class で使える
- Pulley interpreter は pure Rust なので **決定論性を config 調整なしで担保** (JIT 最適化の非決定性が実行層に入らない)
- Cranelift を有効化すれば実行時コンパイルもできる → `.wasm` 配布と `.cwasm` 配布の両方を同じ実装基盤で扱える

###### 2 段 feature 構成

配布フォーマットの選択をビルド時に決めるための feature。

| feature | wasmtime features | 用途 |
|---|---|---|
| `plugin-wasm` (default ON) | `runtime` + `cranelift` + `pulley` + `component-model` + `std` + `wat` + `anyhow` | `.wasm` と `.cwasm` の両方を扱える。実行時コンパイル可能でユーザーが guest を差し替えて試行錯誤できる。Cranelift 分バイナリが大きい |
| `plugin-wasm-runtime-only` (opt-in) | `runtime` + `pulley` + `component-model` + `std` | 事前生成 `.cwasm` の deserialize のみ。Cranelift を落として配布サイズを最小化 |

両 feature とも実行層は Pulley interpreter で共通、インターフェース層は Component Model で共通。差は wasmtime 内の compile 層 (Cranelift) を有効化するかどうかの 1 点のみ。

Phase P0 smoke test で core wasm path (`Module::*`) と Component Model path (`Component::*`) の両方で precompile / deserialize ラウンドトリップを実測確認済み。runtime-only バイナリでの deserialize も含む (実測値は plan 参照)。

###### 複数衛星 lifecycle

- 1 衛星 = 1 `Store<HostState>` + 1 Instance、Engine / Component は Arc で共有
- instantiate cost ~5 µs、100 衛星までは pooling なしで OK
- 1000+ 衛星で pooling allocator を導入 (Phase C)

##### 決定論性の運用ルール

- **`Config::consume_fuel(true)`** で interruption を決定論化 (`epoch_interruption` は禁止、非決定論的)
- **guest/host 両方で `libm` crate 強制** (sin/cos/exp/log 等の host libm 実装差で 1e-12 oracle が破綻するため)
- **guest 側は `HashMap` 禁止、`BTreeMap` のみ** (iteration 順序の決定論)
- **wasmtime / wit-bindgen バージョン pin** (`=x.y.z` で厳格、CI で更新時に oracle 回帰テスト)
- **NaN ガード**: host 側で `Command::is_finite()` を毎 tick チェック (guest の NaN 出力を弾く)

##### Phase P 実装フェーズ

| フェーズ | 内容 |
|---|---|
| Phase P0 | 調査・方針決定 **実装済み**。DESIGN.md 更新 + rust-toolchain.toml pin + smoke test で core wasm / Component Model 両 path の precompile/deserialize ラウンドトリップを確認 (実測値は plan 参照) |
| Phase P0.5 | `NativeController` で trait + adapter + oracle の経路を validation (guest ランタイムを触る前) |
| Phase P1 | wasmtime (Pulley) backend + Detumbling guest (`BdotFiniteDiff` と 1e-12 bit 近似一致) |
| Phase P2 | 第 2 backend 追加 (pure Rust embedded script 系を候補として評価。Phase P1 完了後に選定) |
| Phase P3 | PD 姿勢制御 + モードマシン (detumble → nadir 切替) |
| Phase P4 | 推進 (throttle / impulsive / finite burn) |
| Phase P5 | 姿勢-軌道結合ミッション (1 guest で姿勢 + 推進を同時指令) |
| Phase P6 | CLI/config 統合 (ホットリロードは optional、デフォルトは restart 運用) |

##### 他 Phase との関係

- **Phase E 先行**: guest デバッグに viewer (姿勢表示) が必要
- **Phase D-5 を Phase P1 の前提条件に昇格**: env snapshot (`magnetic-field-eci-t`) のために `MagneticFieldModel` trait 抽象化が先に fix していることが望ましい
- **Phase P5 → Phase C**: 1 guest の扱いを固めてから複数衛星に展開

##### feature gate

- orts library: `plugin-wasm` (default OFF)、`plugin-wasm-runtime-only` (opt-in 代替、Cranelift を抜いた minimal runtime)
- orts-cli: `plugin-wasm` を **default feature に含める**。小サイズ配布したいユーザーは `cargo build --no-default-features --features plugin-wasm-runtime-only` で opt-out 可能
- CI matrix は `{no-plugin, plugin-wasm (=CLI default), plugin-wasm-runtime-only, all-plugins}` + wasmtime バージョン pin 回帰ジョブ
- Rust toolchain は `rust-toolchain.toml` で wasmtime の MSRV に pin

### ミッション規模と力学モデル

問題のスケールに応じて適切なモデルを選択する設計。一つのモデルで全てをカバーしない。

| ミッション | 中心天体 | 必要な天体 | 主な摂動 |
|---|---|---|---|
| LEO (ISS 等) | 地球 (固定) | 月・太陽 | J2+, 大気抵抗, SRP |
| GEO/SSO | 地球 (固定) | 月・太陽 | J2, SRP, 第三体 |
| 月探査 | 地球↔月 | 地球・月・太陽 | 月 J2, 3 体力学 |
| 小惑星探査 | 太陽↔小天体 | 太陽・惑星群 | SRP |
| 外惑星探査 | 太陽↔各惑星 | 太陽・全惑星 | スイングバイ |
| 太陽系シミュレーション | なし (SSB) | 全天体 | 相互重力 |

ユーザーが `OrbitalSystem` に重力場と摂動を組み合わせて問題に応じたモデルを構成する。
ただし、モデルの適用範囲を逸脱した場合はシステムが検知して警告・対応する:

| 状況 | デフォルト動作 |
|---|---|
| 未考慮の天体の摂動が大きくなった | 警告出力 |
| SOI (影響圏) 逸脱 | 警告 + 積分停止 |
| 数値発散 (NaN/Inf) | 積分停止 |
| 大気圏突入 / 衝突 | 積分停止 |

### 惑星間遷移と SOI

中心天体の切り替えが必要な惑星間ミッションへの対応は段階的に進める。

**Phase 1 (現状)**: 中心天体固定。LEO〜GEO は十分カバー。

**Phase 2**: 手動切り替え。イベント検出で SOI 脱出を検知し、積分を停止。
ユーザーが座標変換 + 新しい OrbitalSystem を構築して再開。

**Phase 3**: 自動監視 + 警告。摂動強度比で中心天体の妥当性を継続的に監視。
SOI 境界接近時に警告し、オプションで自動切り替え。

**Phase 4**: 完全 N 体。太陽系規模のシミュレーション用。
慣性系で全天体の重力を直接計算。integrator の State ジェネリック化が前提。

SOI 切り替え時の注意点:
- 第三体重力は差分形式 `a(sc) - a(primary)` で計算し、フレーム切り替えを純粋な座標変換にする
- 切り替え時は積分器をリスタート (FSAL 破棄、刻み幅リセット)
- 地球-月系はネストした SOI が必要 (月は地球 SOI 内)
- ラグランジュ点付近では SOI が破綻するため、摂動強度比ベースの監視で対応

### 設計規約

将来の拡張で手戻りを避けるため、以下を早期に決定する:

- **四元数規約**: Hamilton 規約、スカラー先頭 `(w, x, y, z)`。右手系
- **単位系**: km, km/s, kg (軌道力学の慣例)。SI (m) への変換は明示的に行う
- **座標系規約**: `ExternalLoads` のフィールド名に座標系を明示 (`acceleration_inertial`: 慣性系 ECI [km/s²], `torque_body`: 機体系 [N·m])。座標変換はモデル実装内部で行う
- **Model の純関数性**: `Model<S>::eval(&self, ...)` は副作用を持たない純関数的評価。内部状態の変更は行わない
- **Context パターン**: `DynamicalSystem::derivatives()` に渡す環境情報 (暦元、天体暦、大気モデル) は将来的に Context 構造体に統合する可能性がある。現状は各 system のフィールドで保持
- **trait object ポリシー**: モデルは `Box<dyn Model<ConcreteState>>` で実行時差し替え可能。`GravityField`, `AtmosphereModel` 等の環境 trait も `Box<dyn Trait>` で差し替え可能。`impl GravityField for Box<dyn GravityField>` はビルダーヘルパー用の便利 impl であり、性能クリティカルなパスでは `SpacecraftDynamics<G>` のモノモーフィゼーションを使用する
- **feature gate**: 重いモデル (NRLMSISE-00, Rerun, WebSocket, CSSI HTTP) は feature flag で分離。Rerun は orts の default feature

## Viewer データフローアーキテクチャ

### 設計原則

- **DuckDB-WASM はローカルキャッシュ**: サーバーへのクエリを減らすための履歴ストア。リアルタイム表示のクリティカルパスには置かない
- **live 表示は JS バッファが正**: 3D（TrailBuffer）もチャート（ChartBuffer）もサーバーからのストリーミングデータを直接表示。DuckDB を経由しない
- **derived 値はサーバーで事前計算**: altitude, energy, angular_momentum 等のチャート用 derived 値はサーバーが計算して state メッセージに含める。viewer 側での再計算を排除

### SourceAdapter パイプライン

全データ source (WebSocket / CSV / RRD) は SourceAdapter を通じて統一パイプラインに流れる:

```
SourceAdapter (WS / CSV Worker / RRD WASM Worker)
  │
  └→ SourceEvent (discriminated union)
       │
       useSourceRuntime (event dispatcher)
         ├→ TrailBuffer (3D 用, ref — push で re-render しない)
         │    - OrbitPoint 全体を保持、補間・世代管理
         │    - OrbitTrail GPU shader が直接読み取り
         │
         ├→ ChartBuffer (チャート用、列指向 ring buffer)
         │    - t + metric ごとの Float64Array
         │    - uPlot.setData() に即時反映（DuckDB バイパス）
         │
         └→ IngestBuffer → DuckDB (定期バッチ insert)
              - markRebuild() で初期データ一括投入
              - push() でリアルタイム追加 (両者は同一パイプライン内で共存)
              - 履歴クエリ・zoom 用のキャッシュ
```

CSV ファイルは Web Worker でチャンク単位 (5000行/chunk) にパースし、history-chunk イベントとして投入。main thread をブロックしない。

### チャートデータソースの切り替え

チャートの描画ソースは UI 状態に基づいて選択する:

| 状態 | データソース | 理由 |
|---|---|---|
| live-follow | ChartBuffer (JS) | 最新データを即座に反映 |
| paused / seek | ChartBuffer の coverage 内なら JS、外なら DuckDB | ローカルで解決 |
| zoom（過去方向） | DuckDB (downsampled query) | 履歴の長期トレンドを効率的に表示 |
| ファイル source | IngestBuffer → DuckDB | markRebuild で一括投入、その後は DuckDB クエリ |

切り替え条件: `requestedRange ⊆ chartBuffer.coverage` なら JS バッファ、はみ出したら DuckDB にフォールバック。

### DuckDB の役割

1. **履歴キャッシュ**: 全 state データを蓄積。zoom/seek 時のローカルクエリに使用
2. **downsampling**: 長時間データのバケット分割 + 間引き表示
3. **query_range の代替**: サーバーへの `query_range` リクエストを削減。DuckDB に保持済みの時間帯はローカルで解決し、未保持の時間帯だけサーバーに問い合わせる
4. **compaction**: 古いデータを定期的に間引いてメモリ使用量を制御

### JS バッファと DuckDB の一貫性

完全一致は求めない。「live source が正、DuckDB は eventually consistent cache」と定義する:

- ChartBuffer は直近の全データを full-resolution で保持
- DuckDB は compaction で古いデータが間引かれうる
- source 切替時は ChartBuffer と DuckDB の overlap 区間で stitch（境界の段差を防止）
- live 中は ChartBuffer のみを参照するため、DuckDB の状態は表示に影響しない

### サーバー側 state メッセージの derived 値

チャートに必要な derived 値はサーバーで事前計算する。`state`, `history`, `query_range_response` で共通の構造を使用:

- 軌道要素: `a`, `e`, `inc`, `raan`, `omega`, `nu`（既に含まれている）
- derived: `altitude`, `specific_energy`, `angular_momentum`, `velocity_mag`
- 摂動加速度: 各モデルの加速度ノルム（既に含まれている）

### 再接続時の履歴転送プロトコル

長時間 serve した後の再接続でビューワが真っ白になる問題を踏まえ、履歴転送はシム時間に対して定数コストになるよう設計する:

- **サーバー側**: `HistoryBuffer` が per-entity の adaptively-sampled overview buffer を `push()` 時にインクリメンタルに更新 (amortized O(1))。接続時の `history` メッセージは各衛星につき最大 `OVERVIEW_MAX_POINTS_PER_ENTITY` (= 1000) 点のダウンサンプル済みスナップショットで、ディスク I/O ゼロ、シム時間に非依存の定数コストで返される。`HistoryBuffer::query_range` も in-memory fast path を持ち、直近の window を要求された場合はディスクを触らない。
- **push 型と pull 型の分離**: live state のみ broadcast (push)、過去データの詳細は `query_range` で client が pull する。サーバーは client の time range を知らない — client の表示ウィンドウは client の関心事で、必要な範囲を必要なタイミングで pull する。
- **クライアント側の proactive enrichment**: 再接続後、client は現在の `timeRange` (例: 1h) に対して初期 `query_range` を自発的に投げ、衛星ごとに高解像度データを取得する。サーバーはダウンサンプリングで bounded な応答を返し、`range-response` ハンドラが trail/ingest/chart buffer をまとめて再構築する。

### uneri ライブラリの責務

| 層 | 責務 | 所属 |
|---|---|---|
| ChartBuffer | 列指向 ring buffer、append、getWindow | uneri |
| IngestBuffer | DuckDB ingest 用のステージングバッファ | uneri |
| DuckDB ingest/query | insert, queryDerived, compact | uneri |
| TimeSeriesChart | uPlot ラッパー | uneri |
| source 切替ポリシー | live/paused/zoom でどのソースを使うか | viewer |
| SourceAdapter | WS/ファイルの入力差を吸収し SourceEvent に統一 | viewer |
| useSourceRuntime | adapter 管理、event → buffer routing、source metadata | viewer |