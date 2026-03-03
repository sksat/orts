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
WebSocket サーバーモード。将来的には Web クライアントからシミュレーション条件を指定して実行する機能を持つ。

### convert
フォーマット変換。rrd → csv などをサポート。

## Crate 構成

Rust crate と TypeScript パッケージで構成される。
Rust crate は `orts-` prefix を使用し、ディレクトリ名は prefix なし。
汎用的な独立ライブラリには固有名を付ける（kaname, uneri, tobari）。

### Rust crate

| Crate | ディレクトリ | 責務 |
|---|---|---|
| orts-integrator | `integrator/` | 汎用 ODE ソルバ。OdeState trait、DynamicalSystem trait、RK4、Dormand-Prince 適応刻み |
| kaname | `kaname/` | 測地学・天文ライブラリ。詳細は [`kaname/DESIGN.md`](kaname/DESIGN.md) |
| tobari | `tobari/` | 大気密度モデル・宇宙天気。詳細は [`tobari/DESIGN.md`](tobari/DESIGN.md) |
| orts-orbits | `orbits/` | 軌道力学。重力場 (J2+)、摂動 (抗力/SRP/第三体)、ケプラー要素、TLE/SGP4、イベント検出。orbits→tobari 依存は trait + デフォルト実装のため |
| orts-attitude | `attitude/` | 姿勢力学。AttitudeState (四元数+角速度)、TorqueModel trait、重力傾斜トルク |
| orts-sim | `sim/` | 宇宙機統合。SpacecraftState (軌道+姿勢+質量)、WrenchModel trait、ECS データモデル・Rerun エクスポート |
| orts | `cli/` | CLI + WebSocket サーバ。run/serve/convert サブコマンド |

### TypeScript パッケージ

| パッケージ | ディレクトリ | 責務 |
|---|---|---|
| @orts/uneri | `packages/uneri/` | DuckDB-wasm + uPlot 汎用時系列チャートライブラリ |
| viewer | `viewer/` | リアルタイム 3D 軌道ビューア (React + @react-three/fiber) |

### 依存関係

```
kaname          orts-integrator
  ↑                 ↑
tobari              │
  ↑                 │
orts-orbits ────────┘
  ↑
orts-attitude ──────┘
  ↑
orts-sim (spacecraft + record)
  ↑
orts (CLI/WS)
```

- kaname と orts-integrator は独立（ワークスペース内の他クレートに依存しない）
- tobari は kaname のみに依存し、大気モデルライブラリとして独立性を維持
- orts-orbits は integrator, kaname, tobari を利用
- orts-attitude は integrator, kaname を利用（orbits には依存しない）
- orts-sim は orbits, attitude, integrator, kaname を利用（宇宙機統合 + データ記録）
- orts-integrator は汎用 ODE ソルバであり、軌道力学・姿勢力学の両方から利用される

### 設計原則

- **trait ベースの合成**: GravityField、ForceModel、AtmosphereModel 等の trait でモデルを実行時に差し替え可能
- **問題に応じたモデル構成**: ユーザーが OrbitalSystem に重力場と摂動を組み合わせる。フルスペック計算は不要な場合がほとんど
- **責務の分離**: 各 crate は独立テスト可能。座標変換は kaname、大気は tobari、力学は orbits

## 実装済みのアーキテクチャ拡張

### 姿勢力学 (orts-attitude)

軌道力学 (並進 6 自由度) に加え、姿勢力学 (回転 3 自由度) を扱う crate。

- `AttitudeState`: 四元数 (Hamilton, スカラー先頭) + 角速度 (機体座標系)。7 状態量、OdeState 実装
- `TorqueModel` trait: ForceModel の姿勢版。`torque(t, state, epoch) -> Vector3`
- `GravityGradientTorque`: 重力傾斜トルクモデル
- `AttitudeSystem`: DynamicalSystem 実装。Euler の回転方程式 + 四元数運動学

orts-orbits と orts-attitude は互いに依存しない。
統合も検討したが、姿勢-軌道結合は一時的・シナリオ依存であり opt-in が望ましい。

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

### ForceModel と TorqueModel の意味論

2つの trait は異なる物理方程式に対応し、統一しない:

| | ForceModel | TorqueModel |
|---|---|---|
| 入力状態 | State（位置+速度、慣性系） | AttitudeState（四元数+角速度、機体系） |
| 出力 | 加速度 F/m [km/s²] | トルク τ [N·m] |
| 座標系 | 慣性系 (ECI) | 機体座標系 (body) |
| 質量の扱い | 除算済み | 未除算（Euler方程式で I⁻¹τ） |

姿勢-軌道結合時は LoadModel で両方を並べる設計とする（後述）。
汎用 `VectorModel<State>` への統一も検討したが、物理量・座標系・単位がすべて異なるため意味論が曖昧になる。

## 将来のアーキテクチャ拡張

### orts-sim: 宇宙機型・シミュレーション管理・データ記録

姿勢-軌道結合、複数衛星、衛星分離に対応するためのクレート。
spacecraft 型は結合積分の前から有用（衛星の「位置+姿勢」の統一表現として）なので最初から含める。
独立した orts-spacecraft クレートも検討したが、クレート数の増加を避け、将来必要になったら切り出す方針とした。

```
kaname          orts-integrator
  ↑                 ↑
tobari              │
  ↑                 │
orts-orbits ────────┘
  ↑
orts-attitude ──────┘
  ↑
orts-sim (spacecraft + orchestration + record)
  ↑
orts (CLI/WS) — 薄いワイヤリングのみ
```

#### モジュール構成

```
orts-sim/
  src/
    spacecraft/       # SpacecraftState, ExternalLoads, LoadModel, SpacecraftDynamics
    group/            # PropGroup, GroupState
    scheduler/        # 同期点管理、レジーム遷移
    record/           # Recording, Rerun export (default feature)
    lib.rs
```

#### spacecraft モジュール — 宇宙機の状態と結合力学

**SpacecraftState**: 軌道(6) + 姿勢(7) + 拡張パラメータ（質量/燃料等）。OdeState 実装。
質量/慣性の変化（推進剤消費、分離、デプロイ）に備え拡張可能な設計にする。

**ExternalLoads 値型**: 加速度+トルクを同時に表現。古典的な wrench（同一座標系の force+torque）ではなく、
各フィールドはそれぞれの運動方程式の座標系で表現される。

```rust
struct ExternalLoads {
    acceleration_inertial: Vector3<f64>,  // 慣性系 [km/s²]（→軌道方程式）
    torque_body: Vector3<f64>,            // 機体系 [N·m]（→Euler方程式）
}
```

**LoadModel trait**: `loads(t, state, epoch) -> ExternalLoads`
- SRP/drag/thrust は物理的に力とトルクを同時に生むため、1回の評価で両方を返す
- 座標系変換はモデル実装内部で行う（最も精度良い方法で）

**Adapter** (既存モデルの再利用):
- `ForceModelAtCoM` — ForceModel→LoadModel（torque=0、重心作用を名前で明示）
- `TorqueModelOnly` — TorqueModel→LoadModel（force=0）
- `ForceModelWithLeverArm` — ForceModel + レバーアーム → `τ = r × F`（圧力中心 ≠ 重心の場合）

**SpacecraftDynamics**: DynamicalSystem for SpacecraftState。GravityField + 慣性テンソル + Vec&lt;LoadModel&gt; から合成。
OrbitalSystem + AttitudeSystem をラップせず、プリミティブから直接合成する
（LoadModel が full SpacecraftState を必要とし、既存の ForceModel/TorqueModel の狭い状態型とは互換がないため）。

DynamicalSystem 実装の3層は排他的選択:

| 層 | 型 | 状態 | crate |
|---|---|---|---|
| 軌道のみ | `OrbitalSystem` | State (6D) | orts-orbits |
| 姿勢のみ | `AttitudeSystem` | AttitudeState (7D) | orts-attitude |
| 結合 | `SpacecraftDynamics` | SpacecraftState (14D) | orts-sim |

`*System` は DynamicalSystem trait 実装だが「システム全体の管理」と誤読されうるため、
結合型は `SpacecraftDynamics` と命名した。既存の `*System` 名は当面維持する。

**姿勢依存モデル** (結合積分実装時に追加):
- 姿勢依存ドラグ（投影断面積 + 圧力中心オフセット）
- 姿勢依存SRP（太陽パネル向き）
- スラスタ（推力方向 + 燃料消費）

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
| Phase A | orts-sim 新設: spacecraft モジュール（SpacecraftState, ExternalLoads, LoadModel）+ record モジュール（datamodel 統合）**実装済み** |
| Phase B | 姿勢-軌道結合: SpacecraftDynamics, 姿勢依存 drag/SRP, adapter |
| Phase C | 複数衛星: group + scheduler モジュール、CLI 簡素化 |

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
- **座標系規約**: ForceModel は慣性系 (ECI) で加速度を返す。TorqueModel は機体座標系 (body) でトルクを返す。ExternalLoads 型はフィールド名に座標系を明示 (`acceleration_inertial`, `torque_body`)
- **作用点規約**: ForceModel の出力は重心 (CoM) に作用する加速度。圧力中心≠重心の場合は LoadModel の ForceModelWithLeverArm adapter でトルクに変換
- **Context パターン**: `DynamicalSystem::derivatives()` に渡す環境情報 (暦元、天体暦、大気モデル) は将来的に Context 構造体に統合する可能性がある。現状は OrbitalSystem のフィールドで保持
- **trait object ポリシー**: 拡張ポイント (ForceModel, GravityField, AtmosphereModel, TorqueModel, LoadModel) は `Box<dyn Trait>` で実行時差し替え可能にする
- **feature gate**: 重いモデル (NRLMSISE-00, Rerun, WebSocket, CSSI HTTP) は feature flag で分離。Rerun は orts-sim の default feature