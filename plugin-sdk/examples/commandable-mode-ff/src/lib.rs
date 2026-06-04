//! Commandable mode switching — **fire-and-forget** C&T プロトコル。
//!
//! 地上が `orts.cmd.set-mode.v1` を撃ちっぱなしで送り、FSW は受理可能なら
//! 内部モードを切り替える（応答は返さない）。確認は、FSW が毎 tick downlink
//! する `orts.tlm.mode.v1` テレメトリを **地上が観測** して行う
//! （verify-by-telemetry）。
//!
//! 受理には運用ガードが入る: `nadir` 指向は機体が整定済み（|ω| < ゲート）の
//! ときだけ許可。tumbling 中の nadir 指令は **黙って無視** され、地上は
//! モードテレメトリが変わらないことで失敗を推測する（fire-and-forget なので
//! reason は返さない）。
//!
//! request-response 版 (`commandable-mode-rr`) と同じ `msg-io` transport の
//! 上に載っており、違いは **アプリ層プロトコルだけ** — こちらは応答を返さず、
//! correlation も持たない。
//!
//! C&T の流れを示すのが主題なので、姿勢制御そのものは no-op（前回値を
//! ZOH 保持）に留めている。

use orts_plugin_sdk::bindings::orts::plugin::types::{Command, TickInput};
use orts_plugin_sdk::msg::{self, NodeId, Value};
use orts_plugin_sdk::{Plugin, orts_plugin};

/// 地上 → FSW: モード切替コマンド。
const KIND_SET_MODE: &str = "orts.cmd.set-mode.v1";
/// FSW → 地上: 現在モードのテレメトリ（fire-and-forget の確認手段）。
const KIND_MODE_TLM: &str = "orts.tlm.mode.v1";

/// nadir 受理に必要な角速度ゲート \[rad/s\]。rr 版と同じ閾値。
const NADIR_RATE_GATE_RAD_S: f64 = 0.05;

struct Controller {
    sample_period: f64,
    mode: String,
}

impl Plugin<TickInput, Command> for Controller {
    fn sample_period(&self) -> f64 {
        self.sample_period
    }

    fn init(_config: &str) -> Result<Self, String> {
        Ok(Self {
            sample_period: 1.0,
            // 初期モードは detumble。地上の set-mode で切り替わる。
            mode: "detumble".to_string(),
        })
    }

    fn update(&mut self, input: &TickInput) -> Result<Option<Command>, String> {
        // 1) この tick に届いた uplink コマンドを処理する。
        //    fire-and-forget なので応答は返さない。受理可能なときだけ遷移し、
        //    失敗（不正モード / tumbling 中の nadir）は黙殺する。
        for m in msg::recv_all() {
            if m.kind == KIND_SET_MODE
                && let Some(target) = msg::get_text(&m.payload, "mode")
                && can_enter(target, input)
            {
                self.mode = target.to_string();
            }
        }

        // 2) 現在モードを毎 tick downlink する。地上はこのテレメトリが
        //    目標モードになるまで wait して、コマンドの効果を確認する。
        msg::send_to(
            NodeId::Ground,
            KIND_MODE_TLM,
            msg::key_value([("mode", Value::Text(self.mode.clone()))]),
        );

        // 制御出力はこの例の主題ではない（前回値を ZOH 保持）。
        Ok(None)
    }

    fn current_mode(&self) -> Option<&str> {
        Some(&self.mode)
    }
}

/// 目標モードへ遷移してよいか — この FSW の business logic
/// （fire-and-forget なので bool だけ。reason は返さない）。
///
/// - `detumble`: 常に可。
/// - `nadir`: 機体が整定済み（|ω| < ゲート）のときのみ。tumbling 中は不可。
/// - 未知モード: 不可。
fn can_enter(target: &str, input: &TickInput) -> bool {
    match target {
        "detumble" => true,
        "nadir" => settled(input),
        _ => false,
    }
}

/// 角速度が nadir ゲート未満（整定済み）か。ジャイロ読み値が無ければ
/// レート未確認として保守的に「未整定」とみなす。
fn settled(input: &TickInput) -> bool {
    match input.sensors.gyroscopes.first() {
        Some(g) => {
            let w2 = g.x * g.x + g.y * g.y + g.z * g.z;
            w2 < NADIR_RATE_GATE_RAD_S * NADIR_RATE_GATE_RAD_S
        }
        None => false,
    }
}

orts_plugin!(Controller, mode);
