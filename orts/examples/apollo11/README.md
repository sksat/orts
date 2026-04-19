# Apollo 11 Trajectory Example

Apollo 11 ミッション全行程の軌道シミュレーションと 3D 可視化。

## 動画

https://github.com/sksat/orts/releases/download/v0.1.1/apollo11_combined.mp4

> Overview（上）+ 宇宙船視点（下）の 3 分間アニメーション。
> 地球周回 → TLI → 月遷移 → LOI → 月周回 → TEI → 地球帰還。

## シミュレーション (Rust)

地球中心座標系で全行程を伝播。月・太陽を第三体摂動として扱う。

| イベント | sim GET | 史実 GET | 誤差 |
|---|---|---|---|
| LOI-1 | 75.0h | 75.6h | -0.6h |
| LOI-2 | 79.4h | 80.1h | -0.7h |
| TEI | 135.8h | 135.2h | +0.6h |
| Entry Interface | 196.0h | 195.1h | +0.9h |

参照: NASA Apollo/Saturn V Postflight Trajectory AS-506 (SP-238)

```sh
# シミュレーション実行（RRD 出力）
cargo run --example apollo11 -p orts

# テスト（タイミング・精度の assertion）
cargo test --example apollo11 -p orts
```

## 可視化 (Python + PyVista)

```sh
cd orts/examples/apollo11
uv sync
uv run python plot_3d_pyvista.py --high        # 本番レンダリング (5400 frames, 30fps)
uv run python plot_3d_pyvista.py --low --draft  # 高速プレビュー (900 frames, 5fps)
uv run python plot_3d_pyvista.py --low --frame 90  # 単一フレームデバッグ (GET 90h)
```

### カメラポリシー

`camera_policy.py` — レンダリングから分離されたテスト可能なカメラ制御。

- **地球周回**: 軌道接線 + 地球地平線 pitch + radial-up
- **トランジット**: 近い天体を直視。midpoint で EMA による ~145° 回転平滑化
- **月周回**: 軌道接線 + 月地平線 pitch + radial-up（地平線が常に水平）
- フェーズ間は sigmoid weight blending + EMA smoothing で滑らかに遷移

```sh
uv run pytest test_camera_policy.py -v  # 20 tests
```
