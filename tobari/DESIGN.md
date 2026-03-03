# tobari — 大気密度モデルライブラリ

大気密度モデルと宇宙天気データを提供するライブラリ。

## 責務と範囲

- `AtmosphereModel` trait の定義と実装
- 宇宙天気インフラ（`SpaceWeatherProvider` trait, CSSI パーサ）

## モデル

3つの大気密度モデルを実装。すべて `AtmosphereModel` trait を実装し、`Box<dyn AtmosphereModel>` で実行時に差し替え可能。

| モデル | 入力 | 用途 |
|---|---|---|
| `Exponential` | 高度のみ | 高速・簡易。epoch 不要 |
| `HarrisPriester` | 高度 + 太陽方向 | 昼夜密度変動を考慮 |
| `Nrlmsise00` | 高度 + 位置 + epoch + 宇宙天気 | 完全経験モデル。高精度 |

精度と計算コストのトレードオフがあり、問題に応じてモデルを選択する設計。

## 宇宙天気

NRLMSISE-00 は F10.7 太陽電波フラックスと Ap 地磁気指数を必要とする。

- `SpaceWeatherProvider` trait: 時刻に応じた宇宙天気データを提供する抽象
- `ConstantWeather`: テスト・簡易計算用の定数値プロバイダ
- `CssiSpaceWeather`: CelesTrak CSSI フォーマット（SW-Last5Years.txt）のパーサ
  - `fetch` feature で HTTP ダウンロード + ローカルキャッシュ（24 時間有効）

宇宙天気インフラは NRLMSISE-00 の内部実装ではなく、トップレベルモジュールとして公開。
将来的に他のモデル（JB2008 等）でも利用可能にするため。

## 検証戦略

- pymsis（C リファレンス実装の Python バインディング）による密度 oracle テスト
- Orekit による Harris-Priester / NRLMSISE-00 のクロスバリデーション
- 密度精度: NRLMSISE-00 で max 0.61%, mean 0.09%（vs pymsis）

## データ帰属

- Kp/Ap 地磁気指数: GFZ Helmholtz Centre (CC BY 4.0)
- F10.7 太陽電波フラックス: NOAA SWPC / NRCan (public domain)
- CelesTrak による集約・配信
