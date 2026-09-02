# orts Design Doc

orts は宇宙機シミュレーション (軌道力学 + 姿勢力学) を対象とした数値計算・数値最適化プラットフォーム。

> このドキュメントは設計の **why** (意図、制約、トレードオフ、採らなかった代替案、不変条件) を記述する。
> 現状の構造 (what) は [ARCHITECTURE.md](ARCHITECTURE.md)、未実装の計画は [ROADMAP.md](ROADMAP.md) を参照。

## データモデル

Rerun (https://rerun.io/) のデータフォーマット設計を参考に、ECS (Entity-Component-System) ベースのデータモデルを採用する。

- **Entity**: 階層パスで識別されるオブジェクト (例: `/world/earth`, `/world/sat/iss`)
- **Component**: データの最小単位 (`Position3D`, `Velocity3D` など)
- **Archetype**: Component のバンドル (`OrbitalState`, `CelestialBody` など)

1 つのデータに複数のタイムライン (シミュレーション時刻、ステップ番号、壁時計、カスタム) を紐付けられる。
天体パラメータのような static data はタイムラインを持たず、全時刻で有効とする。

デフォルトの保存形式は Rerun の `.rrd` フォーマット (Apache Arrow IPC ベース、MIT/Apache 2.0 デュアルライセンス) とする。
Rerun SDK を logging-only モードで使用することで、Rerun Viewer からの再解析やクエリも可能にする。

## CLI

CLI はサブコマンド構造 + CWD の orts.toml 自動検出とし、シミュレータ単体の E2E テスト境界として機能させる。
サブコマンドとオプションの一覧は `--help` を参照。

姿勢を扱うかどうかは config から動的に切り替える。
attitude config (慣性テンソル、質量) がなければ OrbitalSystem、あれば SpacecraftDynamics を使う。
軌道のみのユーザーに姿勢の複雑さを見せないための設計判断である。

## crate 構成の方針

crate の一覧と依存レイヤは [ARCHITECTURE.md](ARCHITECTURE.md) と [README.md](README.md) を参照。
arika と tobari の詳細設計はそれぞれの [`arika/DESIGN.md`](arika/DESIGN.md) / [`tobari/DESIGN.md`](tobari/DESIGN.md) にある。

- **命名規約**: orts 固有の crate は `orts-` prefix、ディレクトリ名は prefix なし。汎用的な独立ライブラリには固有名を付ける (arika, utsuroi, tobari, uneri)
- **依存制約**: arika と utsuroi は workspace 内の他 crate に依存しない。tobari は arika のみに依存し、地球周辺環境モデルとして独立性を維持する。依存は常に一方向 (基盤 → 環境 → シミュレーション → アプリ)
- **crate 追加の方針**: crate 数の増加を避け、必要になったら切り出す。spacecraft 型を独立 crate にせず orts に含めたのはこの判断による
- **公開 API の正規パス**: モデル関連の公開 API (`Model`, capability traits, `ExternalLoads`) は `orts::model` を正規のインポートパスとして安定化させる

## 力学アーキテクチャ

### Capability-based Model trait

モデル (摂動力、トルク、スラスタ等) は統一 `Model<S: HasFrame>` trait で表現する。
モデルは state の capability trait (`HasFrame`, `HasOrbit`, `HasAttitude`, `HasMass`) を generic bound で宣言し、bound を満たす全ての system で直接使える。
旧設計 (ForceModel / TorqueModel / LoadModel の 3 本立て) では system 間でモデルを載せ替える際にアダプタが必要で、capability bound はこれを不要にする。

- 慣性系は `HasFrame::Frame` として state に一度だけ宣言し、`HasOrbit` と `HasAttitude` が supertrait として共有する。「orbit は `Gcrs`、attitude は `SimpleEci`」という食い違った宣言は書けない。quaternion の成分は基準 frame に依存し、`SimpleEci` と `Gcrs` は 2024 epoch で 484 秒角ずれるため、この一致は数値の意味そのものを守る
- `eval` の返り値は `ExternalLoads<S::Frame>`。loads の frame は state の frame から来るので、model 側に独立した frame パラメータは無く、「state を読んだ frame と loads を返す frame が異なる」という組み合わせは型として書けない
- **dispatch**: 「このモデルがこの system で使えるか」は capability bound によりコンパイル時に判定する。一方、異種モデルの collection (`Vec<Box<dyn Model<S>>>`) は動的 dispatch とする。ホットパスは ODE 内部ループ (axpy/scale) であり、モデル評価の vtable コストは無視できるため。性能が問題になれば静的 dispatch パスを後から追加できる

### 状態と system の 3 軸

力学の構成は直交する 3 軸で考える。

- **plant**: OrbitalSystem (軌道 6D) / AttitudeSystem (姿勢 7D) / SpacecraftDynamics (結合 14D) を排他的に選択する
- **state extension**: RW 角運動量やジンバル角のような、力学バックリアクションを持つ内部状態 (StateEffector) は `AugmentedState<S>` で ODE 状態を plant 型の外側から拡張する。plant 型は変更しない。名前付きサブステート (AuxRegistry) で raw indexing を局所化する
- **multi-spacecraft regime**: 複数衛星は独立 (各グループが最適 dt) / 同期 (共通同期点で状態交換) / 結合 (単一 ODE) の 3 レジームで扱う。レジーム遷移にはヒステリシスと最小滞留時間を入れ、チャタリングを防ぐ

状態ベクトルは monomorphization を優先する。
`Vec<f64>` ベースの動的状態も検討したが、ODE 内部ループがホットパスであるため棄却した。
可変 N (衛星分離、コンステレーション) は `GroupState<S: OdeState>` の `Vec<S>` で対応し、各衛星の内部演算は固定次元のまま保つ。
ランタイムの構成選択は enum で分岐し、内部は monomorphic に保つ。

### 積分と projection の契約

数値積分を続けると、状態が本来満たすべき制約からのずれが蓄積する (四元数のノルムが 1 からずれる等)。
このずれを制約面に戻す操作 (正規化、clamp) が `OdeState::project` で、いつ呼ぶかを契約として固定している。
制約が厳密解の不変量である限り、数値解の制約からのずれは局所誤差と同じ $O(h^{p+1})$ に留まるため、射影しても収束次数は落ちない (多様体上の ODE に対する標準の projection method。Hairer–Lubich–Wanner, Geometric Numerical Integration, IV.4)。
projection が必要なのは、四元数のように表現自体が制約を持ち、ずれが secular に蓄積する state だけであり、default は no-op。
一方で呼ぶ位置が揺れると、以下のように適応刻みの判定やシンプレクティック性が壊れる。

- 各積分器は「採用したステップの結果」に対してのみ、callback や event 判定に渡す直前に一度だけ project を呼ぶ。reject された候補や RK の中間 stage には呼ばない
- 適応解法の accept/reject 判定は projection 前の生の候補で行う。誤差推定は生の候補に対して計算された量なので、projection 後の状態と組み合わせると判定が非一貫になる
- 戻り値 `Projection::{Unchanged, Changed}` は「実際に状態を書き換えたか」を表す。FSAL (前ステップ終端の微分を次ステップ先頭で再利用する最適化) は状態がそのまま持ち越されることが前提なので、Changed ならキャッシュした微分を破棄する。複合 state は子の結果の OR を返す
- Yoshida (高次シンプレクティック) には汎用 projection を適用しない。正規化や clamp はシンプレクティック写像ではなく、substep の合成の間に挟むと手法の狙いである構造保存が壊れる。そのため projection を持たない Verlet kernel を合成し、Störmer-Verlet 単体は full step の末尾で project を呼んで他の積分器と契約を揃える

### 制御の 3 層

Basilisk や Orekit と同様の 3 層分離を採用する。

| 層 | 状態 | 用途 |
|---|---|---|
| ContinuousModel (`Model<S>`) | なし (純関数) | drag, SRP, gravity gradient, memoryless 制御則 |
| StateEffector | ODE 状態の一部 | RW 角運動量、ジンバル角 (力学バックリアクション) |
| DiscreteController | 内部状態 (`&mut self`) | PID, B-dot (有限差分), フィルタ, モード遷移 |

- **ContinuousModel の境界**: memoryless な計算のみ。サンプル信号、フィルタ、anti-windup、モードロジックは discrete 側に属する
- **DiscreteController**: 固定サンプル周期で segment-by-segment に積分する。制御区間内はコマンドを凍結し (ZOH)、adaptive solver の内部サブステップでも不変とする。共有可変状態 (`Arc<Mutex>`) は使わない

## プラグインアーキテクチャ

宇宙機の制御則やミッションロジックを、host (orts) から分離した外部プラグインとして差し替え可能にする。
再コンパイルなしに制御則を試行錯誤できるようにすることが目的。

### 設計方針

- **guest はサンプル tick でのみ呼ぶ**: guest を ODE RHS のホットパスから完全に外し、tick (segment 境界) でのみ呼び出す。返った command は actuator に ZOH でセットし、次の区間を native Rust で積分する。射程は制御側のみで、ODE RHS で評価される環境モデルのプラグイン化は対象外
- **戻り値は物理量ではなく論理指令**: guest は ExternalLoads (物理量) ではなく per-device のアクチュエータコマンド (MTQ 磁気モーメント、RW 速度/トルク、スラスタ throttle) を返す。物理化 (トルク・力への変換) と 2 段階 clamp (ドライバ受付範囲 / 物理制約) は Rust 側の actuator assembly が担う。plugin が actuator allocation まで担当することで、実機の flight software に近い制御を書ける
- **環境情報は tick 開始時の snapshot を一括渡し**: host が tick 開始時に TickInput (真値 state、デバイスごとのセンサ読み値、actuator テレメトリ、epoch) を確定して渡す。磁場のような一部の値は host-env import による on-demand 取得も残す。WASM 境界の marshalling は WIT の Canonical ABI に任せる (独自シリアライズ (postcard) 案は、WIT が型と互換性を管理できるため棄却)
- **既存 Rust 実装は削除せず oracle に使う**: 同じロジックの Rust 実装とプラグインで同等性を検証する。精度契約は 2 種類を区別する: native Rust ↔ WASM は tolerance ベース (現行 1e-4)、sync backend ↔ async backend は bit-exact。backend 追加時は同等性テストを pass しないとマージしない (CI enforce)
- **ミッションモードマシンは 1 プラグイン内部で分岐**: モード切替 (Detumble → Nadir → Burn) は host が guest を差し替えるのではなく、guest 内部で分岐する。guest は current-mode で現在モードを公開する (現状は run が Store を占有するため並行取得に制約がある)

### WASM backend

インターフェース契約は Component Model + WIT に置く (`orts/wit/`)。
record / variant などの高級型を宣言的に記述でき、wit-bindgen が多言語の guest bindings を生成する。
orts の長期的な API 契約として安定化させる対象はこの WIT であり、破壊的変更は version directory (`v0`, ...) で互換性を管理する。

ランタイムは wasmtime + Pulley interpreter を採用する。

- Pulley は pure Rust の portable interpreter で、JIT 最適化の非決定性が実行層に入らない。決定論を config 調整なしで担保できる
- Component Model と wit-bindgen を first-class で使える (wasmi を採用しなかったのは Component Model 非対応のため)
- 配布は portable な `.wasm` (実行時に Cranelift で compile)。事前コンパイル済み `.cwasm` の配布は [ROADMAP.md](ROADMAP.md)

### 決定論性の運用ルール

- guest / host 両方で libm crate を強制する (sin/cos 等の host libm 実装差で oracle が破綻するため)
- guest 側は HashMap 禁止、BTreeMap のみ (iteration 順序の決定論)
- wasmtime / wit-bindgen は minor 固定で pin し、更新時に oracle 回帰テストを回す
- host 側で `Command::is_finite()` を毎 tick チェックし、guest の NaN 出力を弾く

### sync / async デュアルバックエンド

WASM guest を駆動するホスト実装は sync (OS thread + channel) と async (tokio task + fiber) の 2 つが並存する。
どちらも同じ trait (PluginController) を実装するので、呼び出し側からは透過。
per-tick コストは同等で、async の優位は per-satellite メモリにある (OS thread の MB 級 stack に対し task は KB 級)。

- **決定論契約**: 順次呼び出し + single worker で sync / async は bit-for-bit 同じ結果を出す (oracle で常時検証)。multi-worker の throughput mode は決定論契約の外にある別モード
- **選択**: CLI の `--plugin-backend sync|async|auto`。auto は衛星数の閾値で切り替える
- **呼び出しコンテキスト制約**: async backend の update は内部で `Handle::block_on` を使うため、tokio async タスクの中から直接呼ぶと panic する。平の同期コードか `spawn_blocking` から呼ぶこと (`orts serve` の sim loop は `spawn_blocking` に退避済み)

なお、plugin 層の PluginController と既存の DiscreteController は意図的に並存している (plugin 側は Command 型と TickInput を固定し、WASM guest と native 実装で同じ形を共有するため)。
trait の統一は [ROADMAP.md](ROADMAP.md)。

### ノード間メッセージング (msg-io)

地上局 ↔ FSW と将来の衛星間通信のメッセージを運ぶ transport 層。
制御ループ (tick-io) とは独立させる。

通信は常に component ↔ component と考え、FSW が見る口 (port kind) と、その口が何に配線されているか (connection) を分ける。

- **port kind**: tick-io (制御出力の control plane) / msg-io (node ↔ node の datagram service) / stream-io (順序バイトの conduit)
- **connection の属性**: scope (intra-node / inter-node)、impairment (lossless / impaired)、time model (deterministic-tick / replay / live)。これは config 軸を増やす話ではなく思考の枠

transport は dumb pipe とし、配送とアドレッシングのみを担う (payload は解釈しない)。
fire-and-forget や request-response といった interaction model はアプリ層 (payload + SDK ヘルパ) に置く。
こうすると同じ WIT 契約の上に複数モデルを載せられ、モデルを差し替えても契約は変わらない。

データモデルは論理型 kind (content-type、名前空間 + version 規約) と payload のエンコーディング (key-value / binary / json) を分離する。
envelope は最小限に保ち、host 採番メタ (seq、配送 tick 等) はそれを読む consumer が現れるまで足さない (YAGNI)。

配送意味論:

- **受信**: host が tick 境界で inbox を凍結し、guest が recv-batch で drain する。tick 途中の新着は次 tick へ回るので、いつ何回呼んでも観測は不変 (決定論)
- **送信**: append。src は host が注入する (なりすまし防止)

非決定論の発生源は対話入力 (WebSocket) のみ。
host が tick 境界で gate して「どの tick で何を配ったか」を確定すれば、config の `[[command]]` と同形のコマンドタイムラインになり、それを流し直せば全体が bit-for-bit 再現する。
録った運用セッションがそのまま oracle / 回帰テストになる。

### バイトストリーム接続 (stream-io)

実機通信は「上位 = パケット / 下位 = バイトストリーム」のことが多く、ArkEdge では各コンポーネントを [`arkedge/kble`](https://github.com/arkedge/kble) (virtual harness) で配線している。
orts をその harness に組み込むための named byte stream の口が stream-io。
純 sim では msg-io を、実機ツール統合では stream-io を使い分ける。

- **dumb byte conduit**: orts は中身を解釈しない。framing (EB90 / C2A / CCSDS 等) やプロトコル解釈は FSW 側 + kble pipeline の責務。射程はバイトストリーム層まで (RF/PHY は対象外)
- **決定論**: host が tick 境界で受信バッファを凍結し送信を flush する。足りないフレームは次 tick へ持ち越して FSW が再組立する
- **no-data と closed の区別**: 空 `list<u8>` では「今 tick データ無し」と「相手切断」を判別できないため、result 型で区別する
- **no-drop 契約**: bounded queue が溢れたら byte を drop せず overrun とし、host が authoritative に sim を停止する (drop はフレーム破壊を隠すため)
- **named streams**: guest は自分の local 名 (`"comlink"` 等) だけを見る。host が外部への対応づけを持ち、guest から他衛星の stream は見えない
- **live ブリッジ**: stream ごとに素の binary WebSocket endpoint を公開する (kble 専用プロトコルにしない。binary WS を喋るものなら何でも繋がる)。WS 切断は transient、同一 stream への新接続は後勝ち。stdio ブリッジは専有ケーブルで、config reload は透過再 attach (FSW から見て連続リンク。WS の「再接続 = 新ストリーム」と意図的に非対称)
- 将来の RF はこの byte seam の外に別 sideband (modem のデバイス模型化) で扱う。`stream-read` を variant にしてあるのは gap/erasure イベントを足す余地のため

## ミッション規模と力学モデル

常にフルスペックの計算をするのは過剰なので、系や計算ロジックや精度は切り替え可能な設計とし、一つのモデルで全てをカバーしない。
例えば SSO の計算にはせいぜい地球 - 月 - 太陽の系があればよく、大気抵抗も詳細モデルと抵抗係数のみの簡易モデルを選べるようにする。

| ミッション | 中心天体 | 必要な天体 | 主な摂動 |
|---|---|---|---|
| LEO (ISS 等) | 地球 (固定) | 月・太陽 | J2+, 大気抵抗, SRP |
| GEO/SSO | 地球 (固定) | 月・太陽 | J2, SRP, 第三体 |
| 月探査 | 地球↔月 | 地球・月・太陽 | 月 J2, 3 体力学 |
| 小惑星探査 | 太陽↔小天体 | 太陽・惑星群 | SRP |
| 外惑星探査 | 太陽↔各惑星 | 太陽・全惑星 | スイングバイ |
| 太陽系シミュレーション | なし (SSB) | 全天体 | 相互重力 |

モデルの適用範囲を逸脱した場合はシステムが検知して積分を停止する。
現在検知するのは数値発散 (NaN/Inf) と大気圏突入 / 衝突。
未考慮天体の摂動監視と SOI (影響圏) 逸脱の検知は未実装 ([ROADMAP.md](ROADMAP.md) の惑星間遷移)。

中心天体の切り替えが必要な惑星間ミッションへの対応は段階的に進める ([ROADMAP.md](ROADMAP.md) 参照)。
切り替えを実装する際の設計制約:

- 第三体重力は差分形式 `a(sc) - a(primary)` で計算し、フレーム切替を純粋な座標変換にする
- 切替時は積分器をリスタートする (FSAL 破棄、刻み幅リセット)
- 地球-月系はネストした SOI が必要 (月は地球 SOI 内)
- ラグランジュ点付近では SOI が破綻するため、摂動強度比ベースの監視で対応する

## 設計規約

- **四元数規約**: Hamilton 規約、スカラー先頭 `(w, x, y, z)`。右手系
- **単位系**: km, km/s, kg (軌道力学の慣例)。SI (m) への変換は明示的に行う
- **座標系の型付け**: フィールド名と型で座標系を明示する。`ExternalLoads<S::Frame>` の acceleration は慣性系 [km/s²]、torque は機体座標系 [N·m]。座標変換はモデル実装の内部で行う
- **ExternalLoads の不変条件**: acceleration / torque / mass_rate は加算的に合成する。全モデルは同一の immutable state snapshot に対して評価され、評価順序に依存しない
- **Model の純関数性**: `eval(&self, ...)` は副作用を持たない。内部状態が必要な計算は、力学バックリアクションを持つ連続状態なら StateEffector に、フィルタやモード遷移などの離散状態なら DiscreteController に置く
- **モデル登録の責務**: 環境モデルの登録は `orts::setup` に集約する。CLI (`orts run` / `orts serve`) は `SatelliteParams` を組み立てて渡すだけにする。エントリポイントごとに登録すると、二つが食い違ったときにどちらが正しいのか判断できない。actuator (RW, MTQ, thruster) は機体のハードウェア記述で決まるので、登録は呼び出し側に残す
- **外乱トルクの選択**: どの外乱トルクを解くかは config (`[satellites.disturbances]`) で選ぶ。姿勢を持たない衛星にトルクは install しないので、`[satellites.attitude]` 不在での指定は reject する。姿勢の状態と機体特性を述べる table とは別に置くのは、どの環境モデルを解くかが別の関心事だから
- **パネル形状は SRP と大気抵抗で共有する**: 機体の外形は一つなので、`[[satellites.panels]]` に書いたパネルは両方のモデルが使う。片方だけをパネルで表して他方を等方面で表す口は作らない。等方面のパラメータ (`srp_area_to_mass`, `ballistic_coeff`) との同時指定は、同じ力を二通りに述べていてどちらを使うか読めないので reject する
- **パネルの輪郭は optional**: パネルは面積と法線と圧力中心で表し、面内の輪郭は持たなくてよい。力の法則が輪郭を使わないからである。輪郭を書いたパネル同士だけが遮蔽の判定対象になる。遮蔽は完全に隠れる場合だけを扱う。一部だけ隠れる場合は圧力中心が照射領域の重心に移り、定数の `cp_offset` では表せない
- **trait object ポリシー**: モデルや環境 trait (`GravityField`, `AtmosphereModel` 等) は `Box<dyn Trait>` で実行時差し替え可能とする。性能クリティカルなパスでは generic パラメータの monomorphization を使う
- **feature gate**: 重いモデルや I/O (NRLMSISE-00, Rerun, WebSocket, CSSI HTTP, plugin-wasm) は feature flag で分離する

## Viewer データフローアーキテクチャ

viewer への入力 (WebSocket ストリーム、CSV / RRD ファイル) は、すべて **Source** という単一の抽象として扱い、同じパイプラインに流す。
入力源ごとのモード切替を持たないのは、入力の差を adapter 層で吸収しておけば、複数 source の同時接続・比較表示 (将来) に自然に拡張できるためである。
データフローの図と経路は [ARCHITECTURE.md](ARCHITECTURE.md) を参照。

### 設計原則

- **DuckDB-WASM はローカルキャッシュ**: サーバーへのクエリを減らすための履歴ストア。リアルタイム表示のクリティカルパスには置かない
- **live 表示は JS バッファが正**: 3D (TrailBuffer) もチャート (ChartBuffer) もストリーミングデータを直接表示し、DuckDB を経由しない
- **derived 値はサーバーで事前計算**: altitude, energy 等のチャート用 derived 値はサーバーが計算して state メッセージに含め、viewer 側での再計算を排除する

### チャートデータソースの切り替えポリシー

| 状態 | データソース | 理由 |
|---|---|---|
| live-follow | ChartBuffer (JS) | 最新データを即座に反映 |
| paused / seek | coverage 内なら JS、外なら DuckDB | ローカルで解決 |
| zoom (過去方向) | DuckDB (downsampled query) | 長期トレンドを効率的に表示 |
| ファイル source | DuckDB | 一括投入後はクエリで解決 |

切り替え条件: `requestedRange ⊆ chartBuffer.coverage` なら JS バッファ、はみ出したら DuckDB にフォールバック。

### 一貫性の定義

JS バッファと DuckDB の完全一致は求めない。
「live source が正、DuckDB は eventually consistent cache」と定義する。
DuckDB は compaction で古いデータが間引かれうる。
source 切替時は overlap 区間で stitch し、境界の段差を防ぐ。

### 再接続時の履歴転送

履歴転送はシム時間に対して定数コストになるよう設計する。

- 接続時の history はダウンサンプル済み overview (per-entity 上限つき) を返し、シム時間に依存しない
- live state のみ push し、過去データの詳細は client が query_range で pull する。サーバーは client の表示ウィンドウを知らない (必要な範囲は client が必要なタイミングで pull する)
- 再接続後は client が現在の表示範囲に対して自発的に query_range を投げ、高解像度データを取得する

### uneri の責務境界

| 責務 | 所属 |
|---|---|
| ChartBuffer / IngestBuffer / DuckDB ingest・query / TimeSeriesChart (汎用時系列基盤) | uneri |
| source 切替ポリシー / Source layer / useSourceRuntime (orts 固有のデータ配線) | viewer |
