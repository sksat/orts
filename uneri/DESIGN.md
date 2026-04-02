# uneri Design Doc

uneri は DuckDB-WASM + uPlot ベースの汎用時系列チャートライブラリ。
スキーマ駆動設計で、カラム定義と derived SQL 式を宣言するだけで、データ取り込み・downsampling・チャート描画を処理する。

## 設計原則

- **スキーマ駆動**: `TableSchema` で base columns + derived SQL を宣言。テーブル作成・insert・query・chart 列定義が全て schema から自動生成される
- **DuckDB はローカルキャッシュ**: リアルタイム表示のクリティカルパスには置かない。履歴クエリ・downsampling・compaction 用のストア
- **リアルタイムは JS バッファ直結**: ChartBuffer（列指向 ring buffer）から uPlot.setData() へ直接渡す。DuckDB のクエリレイテンシをバイパス
- **derived は row-local only**: `DerivedColumn.sql` は行ローカルな式のみ（window 関数・集約・相関サブクエリは不可）。incremental query の正当性を保証するための制約

## アーキテクチャ

```
データ到着（push/pushMany）
  │
  ├→ ChartBuffer (列指向 ring buffer)
  │    - t + metric ごとの Float64Array
  │    - live 表示用: WS 到着 → 即座に setData()
  │    - getWindow(tMin, tMax) で表示範囲を返す
  │
  └→ IngestBuffer → DuckDB (バッチ insert)
       - useTimeSeriesStore の tick ループで定期的に drain → INSERT
       - 表示には使わない（バックグラウンド蓄積）
       - 履歴クエリ: queryDerived (downsampled) / queryDerivedIncremental (hot path)
       - compaction: 古いデータを間引いてメモリ制御
```

### データ投入パターン

IngestBuffer は以下の投入パターンを全てサポートする:

| パターン | API | 例 |
|---|---|---|
| ストリーミング | `push()` 逐次 | WS からの state メッセージ |
| 一括投入 | `markRebuild(points)` | ファイル読み込み、接続時の history |
| 一括 + ストリーミング | `markRebuild()` → `push()` | 過去データ表示しつつリアルタイム追加 |
| チャンク投入 | `push()` を繰り返し | Worker からのチャンクパース結果 |

`markRebuild(data)` 後の `push(point)` は `point.t > max(data.t)` でなければならない（時刻単調増加契約）。

### チャートデータソースの選択

| 状態 | データソース |
|---|---|
| live-follow | ChartBuffer (JS) |
| paused / seek（coverage 内） | ChartBuffer (JS) |
| zoom（coverage 外） | DuckDB (downsampled query) |
| 一括投入済み / ストリーミング停止後 | DuckDB |

切り替え条件: `requestedRange ⊆ chartBuffer.coverage` なら JS バッファ、はみ出したら DuckDB。

## モジュール構成

```
uneri/src/
  types.ts              # TimePoint, ColumnDef, DerivedColumn, TableSchema, ChartDataMap
  index.ts              # public exports

  db/
    duckdb.ts           # DuckDB-WASM 初期化
    IngestBuffer.ts     # drain パターンのステージングバッファ
    store.ts            # SQL ビルダー + クエリ実行 (create/insert/query/compact)

  hooks/
    useDuckDB.ts        # DuckDB 接続管理 hook
    useTimeSeriesStore.ts  # cold/hot tick ループ hook

  components/
    TimeSeriesChart.tsx # uPlot ラッパー (programmatic update guard 付き)

  utils/
    chartViewport.ts    # binary search (lowerBound/upperBound) + viewport slicing
    mergeChartData.ts   # cold + hot マージ + left-edge trim
    alignTimeSeries.ts  # multi-table 時系列アライメント
```

## 型設計

### TableSchema

```typescript
interface TableSchema<T extends TimePoint> {
  tableName: string;
  columns: ColumnDef[];        // DuckDB に保存する base columns
  derived: DerivedColumn[];    // SELECT 時に SQL 式で計算する派生量
  toRow(point: T): (number | null)[];
}
```

- `columns` はデータの物理表現（DuckDB テーブル定義）
- `derived` はクエリ時の論理表現（SQL SELECT 式）
- `toRow` は型 T → SQL VALUES 行への変換

### ChartDataMap

```typescript
type ChartDataMap = {
  t: Float64Array;
  [derivedName: string]: Float64Array;
};
```

queryDerived / queryDerivedIncremental / mergeChartData の共通出力型。
uPlot の AlignedData に変換して描画。

## DuckDB クエリ戦略

### Full query (queryDerived) — cold path

時間バケット分割 + ROW_NUMBER による downsampling。maxPoints 件に間引く:

1. `WHERE t >= tMin` でフィルタ
2. 時間範囲を maxPoints 個の等間隔バケットに分割
3. 各バケットの最初の点を選択
4. 最新点を UNION で保証

計算コストが高いため、5-10 秒間隔または明示的トリガーで実行。

### Incremental query (queryDerivedIncremental) — hot path

downsampling なしの軽量クエリ。`WHERE t > coldTMax ORDER BY t`。
毎 tick 実行。cold snapshot 以降の新着データのみ取得。

### cold/hot マージ

```
cold (downsampled, ~2000 点) + hot (full-res, ~数十点)
  → mergeChartData() で concat
  → trimChartDataLeft() で左端を O(1) trim (subarray)
  → setData()
```

### Compaction

長時間実行時のメモリ制御。NTILE バケットで古いデータを間引く:
- `maxRows` 超過で発火
- `keepRecentRows` 件は full-resolution で保持
- 残りを `targetOldRows` 件に downsampling

## 契約

### t の単調増加

`IngestBuffer` に push されるデータの `t` は strictly increasing でなければならない。
hot path の `WHERE t > coldTMax` による重複回避、`lowerBound` によるバイナリサーチ、cold + hot の単純 concat が全てこの前提に依存する。

dev-mode assert で違反を検出する。

### derived は row-local

`DerivedColumn.sql` は同一行のカラムのみを参照する式でなければならない。
`LAG`, `LEAD`, `AVG(...) OVER`, `ROW_NUMBER OVER` 等の window 関数は不可。
incremental query で部分的な行集合に対して評価しても、full query と同じ結果になることを保証するための制約。
