//! Commandable mode switching — **fire-and-forget** C&T プロトコル。
//!
//! 地上が `orts.cmd.set-mode.v1` を撃ちっぱなしで送り、FSW は受理して
//! 内部モードを切り替えるだけ（応答は返さない）。確認は、FSW が毎 tick
//! downlink する `orts.tlm.mode.v1` テレメトリを **地上が観測** して行う
//! （verify-by-telemetry）。
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

/// この FSW が受理する有効なモード名。
const VALID_MODES: [&str; 2] = ["detumble", "nadir"];

/// 地上 → FSW: モード切替コマンド。
const KIND_SET_MODE: &str = "orts.cmd.set-mode.v1";
/// FSW → 地上: 現在モードのテレメトリ（fire-and-forget の確認手段）。
const KIND_MODE_TLM: &str = "orts.tlm.mode.v1";

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

    fn update(&mut self, _input: &TickInput) -> Result<Option<Command>, String> {
        // 1) この tick に届いた uplink コマンドを処理する。
        //    fire-and-forget なので応答は返さない。
        for m in msg::recv_all() {
            if m.kind == KIND_SET_MODE
                && let Some(target) = msg::get_text(&m.payload, "mode")
            {
                handle_set_mode(&mut self.mode, target);
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

/// set-mode コマンドの処理 — この FSW の business logic。
///
/// fire-and-forget では応答を返さないので、地上は「テレメトリでモードが
/// 変わったか」でしか結果を知れない。よって不正なモード名は黙って無視する
/// （= テレメトリのモードが変わらないことで、地上は失敗を推測する）。
///
/// 運用判断の余地: 有効モード集合、遷移可否（例 tumbling 中に nadir 直行を
/// 許すか）、無視 vs ログ出力。ここではシンプルに「既知モードなら遷移」。
fn handle_set_mode(current: &mut String, target: &str) {
    if VALID_MODES.contains(&target) {
        *current = target.to_string();
    }
}

orts_plugin!(Controller, mode);
