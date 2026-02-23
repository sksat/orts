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
| orts-integrator | `integrator/` | 汎用 ODE ソルバ。DynamicalSystem trait、RK4、Dormand-Prince 適応刻み |
| kaname | `kaname/` | 座標変換・暦元・天体暦。ECI/ECEF/WGS-84 変換、Epoch (Julian Date)、太陽・月位置 |
| tobari | `tobari/` | 大気密度モデル・宇宙天気。Exponential / Harris-Priester / NRLMSISE-00、CSSI パーサ |
| orts-orbits | `orbits/` | 軌道力学。重力場 (J2+)、摂動 (抗力/SRP/第三体)、ケプラー要素、TLE/SGP4、イベント検出 |
| orts-datamodel | `datamodel/` | ECS 風データモデル・Rerun (.rrd) エクスポート |
| orts | `cli/` | CLI + WebSocket サーバ。run/serve/convert サブコマンド |

### TypeScript パッケージ

| パッケージ | ディレクトリ | 責務 |
|---|---|---|
| @orts/uneri | `packages/uneri/` | DuckDB-wasm + uPlot 汎用時系列チャートライブラリ |
| viewer | `viewer/` | リアルタイム 3D 軌道ビューア (React + @react-three/fiber) |

### 依存関係

```
orts-integrator (nalgebra)     kaname (nalgebra)     orts-datamodel (nalgebra, rerun)
                  \                |
                   \            tobari (kaname)
                    \           /
                  orts-orbits (integrator, kaname, tobari)
                        |              |
                       orts (CLI)  ----+
```

orts-integrator と kaname は独立。両方を orts-orbits が利用する。
orts-integrator は軌道力学の知識を持たない汎用的な ODE ソルバであり、将来的に姿勢力学等からも利用される。

### 設計原則

- **trait ベースの合成**: GravityField、ForceModel、AtmosphereModel 等の trait でモデルを実行時に差し替え可能
- **問題に応じたモデル構成**: ユーザーが OrbitalSystem に重力場と摂動を組み合わせる。フルスペック計算は不要な場合がほとんど
- **責務の分離**: 各 crate は独立テスト可能。座標変換は kaname、大気は tobari、力学は orbits

## 将来のアーキテクチャ拡張

### 姿勢制御 (ADCS)

軌道力学 (並進 6 自由度) に加え、姿勢力学 (回転 3 自由度) を扱うための crate を追加する。

- **orts-attitude** (新規): 姿勢力学 crate
  - `AttitudeState`: 四元数 + 角速度 (7 状態量)
  - `TorqueModel` trait: ForceModel の姿勢版
  - 実装: 重力傾斜トルク、磁気トルク、リアクションホイール、スラスタ
  - `AttitudeSystem`: OrbitalSystem に対応する姿勢力学の合成器

- **orts-spacecraft** (新規): 軌道と姿勢の結合層
  - `SpacecraftProperties`: 質量、慣性テンソル、形状、反射率
  - `CoupledState`: 軌道 + 姿勢の 13 自由度結合状態
  - 結合効果: 姿勢→抗力断面積/SRP 反射面、軌道→重力傾斜トルク
  - 非結合/弱結合/強結合をモードで選択可能

依存関係 (循環なし):

```
orts-integrator
    ├── orts-orbits (integrator, kaname, tobari)
    ├── orts-attitude (integrator, kaname)        ← 新規
    └── orts-spacecraft (orbits, attitude)         ← 新規
```

orts-orbits と orts-attitude は互いに依存せず、orts-spacecraft が橋渡しする。

### Integrator のジェネリック化

現在の integrator は `State = (Vector3, Vector3)` 固定。姿勢・結合・N体に対応するためジェネリック化する。

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

汎用 ODE ソルバでは状態と導関数は同じ型 (d/dt [q, q'] = [q', q''])。
RK4、Dormand-Prince は OdeState の trait メソッドのみを使い、具体的な型を知らない。
現在の `State` struct はそのまま `OdeState` を実装する。

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
- **Context パターン**: `DynamicalSystem::derivatives()` に渡す環境情報 (暦元、天体暦、大気モデル) は将来的に Context 構造体に統合する可能性がある。現状は OrbitalSystem のフィールドで保持
- **trait object ポリシー**: 拡張ポイント (ForceModel, GravityField, AtmosphereModel, TorqueModel) は `Box<dyn Trait>` で実行時差し替え可能にする
- **feature gate**: 重いモデル (NRLMSISE-00, Rerun, WebSocket, CSSI HTTP) は feature flag で分離