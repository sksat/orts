//! Commandable mode switching — **request-response** C&T プロトコル。
//!
//! 地上が `orts.cmd.set-mode.v1` を送る際、payload に `req-id` を入れる。
//! FSW は受理/拒否を判定し、`orts.ack.set-mode.v1` を **同じ req-id を
//! echo** して返す。correlation は transport ではなく **アプリ層**（payload
//! の中身）で持つ — これがレイヤ分離の要点。
//!
//! 受理判定には運用ガードを入れている: `nadir` 指向は機体が整定済み
//! （|ω| < ゲート）のときだけ許可し、tumbling 中は拒否する（detumble を
//! 先に通させる ADCS 安全インターロック）。reject 時は `reason` を返す。
//!
//! fire-and-forget 版 (`commandable-mode-ff`) と同じ `msg-io` transport の
//! 上に載っており、違いは「応答を返すか／correlation を持つか」だけ。

use orts_plugin_sdk::bindings::orts::plugin::types::{Command, TickInput};
use orts_plugin_sdk::msg::{self, NodeId, Payload, Value};
use orts_plugin_sdk::{Plugin, orts_plugin};

/// 地上 → FSW: モード切替リクエスト（payload に `req-id`）。
const KIND_SET_MODE: &str = "orts.cmd.set-mode.v1";
/// FSW → 地上: リクエストへの応答（同じ `req-id` を echo）。
const KIND_SET_MODE_ACK: &str = "orts.ack.set-mode.v1";

/// nadir 受理に必要な角速度ゲート \[rad/s\]。これを上回る間は tumbling と
/// みなして nadir 指令を拒否する。実機では機体ごとに config で調整する想定。
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
            mode: "detumble".to_string(),
        })
    }

    fn update(&mut self, input: &TickInput) -> Result<Option<Command>, String> {
        // この tick に届いたリクエストを処理し、それぞれに応答を返す。
        for m in msg::recv_all() {
            if m.kind == KIND_SET_MODE {
                self.handle_set_mode(&m.payload, input);
            }
        }
        Ok(None)
    }

    fn current_mode(&self) -> Option<&str> {
        Some(&self.mode)
    }
}

impl Controller {
    /// set-mode リクエストを処理し、correlation 付き ack を返す
    /// — この FSW の business logic。
    ///
    /// request-response では、受理でも拒否でも **必ず ack を返す**。ack は
    /// 元リクエストの `req-id` を echo するので、地上は「どのコマンドの結果か」
    /// を対応付けられる。reject 時は `reason` も載せる。
    fn handle_set_mode(&mut self, payload: &Payload, input: &TickInput) {
        // correlation id。欠落していても動く（その場合は -1 を echo）。
        let req_id = match msg::get(payload, "req-id") {
            Some(Value::Integer(id)) => *id,
            _ => -1,
        };
        let target = msg::get_text(payload, "mode").unwrap_or("");

        let (status, reason) = evaluate_transition(target, input);
        if status == "accepted" {
            self.mode = target.to_string();
        }

        msg::send_to(
            NodeId::Ground,
            KIND_SET_MODE_ACK,
            msg::key_value([
                ("req-id", Value::Integer(req_id)),
                ("status", Value::Text(status.to_string())),
                ("reason", Value::Text(reason.to_string())),
                // 受理なら新モード、拒否なら現モード（変更なし）。
                ("mode", Value::Text(self.mode.clone())),
            ]),
        );
    }
}

/// 運用ガード本体 — 目標モードへの遷移可否と理由を返す。
///
/// - `detumble`: 常に受理。
/// - `nadir`: 機体が整定済み（|ω| < ゲート）のときのみ受理。tumbling 中は
///   `"still-tumbling"` で拒否（detumble を先に通させる安全インターロック）。
/// - 未知モード: `"unknown-mode"` で拒否。
///
/// 運用判断の余地: ゲート閾値、ガード対象の遷移、理由コード体系。ここは
/// あなたのドメイン（ADCS 運用）で調整する想定の差し替え点。
fn evaluate_transition(target: &str, input: &TickInput) -> (&'static str, &'static str) {
    match target {
        "detumble" => ("accepted", ""),
        "nadir" => {
            if settled(input) {
                ("accepted", "")
            } else {
                ("rejected", "still-tumbling")
            }
        }
        _ => ("rejected", "unknown-mode"),
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
