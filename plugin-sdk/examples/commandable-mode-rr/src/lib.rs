//! Commandable mode switching — **request-response** C&T プロトコル。
//!
//! 地上が `orts.cmd.set-mode.v1` を送る際、payload に `req-id` を入れる。
//! FSW は受理/拒否を判定し、`orts.ack.set-mode.v1` を **同じ req-id を
//! echo** して返す。correlation は transport ではなく **アプリ層**（payload
//! の中身）で持つ — これがレイヤ分離の要点。
//!
//! fire-and-forget 版 (`commandable-mode-ff`) と同じ `msg-io` transport の
//! 上に載っており、違いは「応答を返すか／correlation を持つか」だけ。

use orts_plugin_sdk::bindings::orts::plugin::types::{Command, TickInput};
use orts_plugin_sdk::msg::{self, NodeId, Payload, Value};
use orts_plugin_sdk::{Plugin, orts_plugin};

/// この FSW が受理する有効なモード名。
const VALID_MODES: [&str; 2] = ["detumble", "nadir"];

/// 地上 → FSW: モード切替リクエスト（payload に `req-id`）。
const KIND_SET_MODE: &str = "orts.cmd.set-mode.v1";
/// FSW → 地上: リクエストへの応答（同じ `req-id` を echo）。
const KIND_SET_MODE_ACK: &str = "orts.ack.set-mode.v1";

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

    fn update(&mut self, _input: &TickInput) -> Result<Option<Command>, String> {
        // この tick に届いたリクエストを処理し、それぞれに応答を返す。
        for m in msg::recv_all() {
            if m.kind == KIND_SET_MODE {
                handle_set_mode(&mut self.mode, &m.payload);
            }
        }
        Ok(None)
    }

    fn current_mode(&self) -> Option<&str> {
        Some(&self.mode)
    }
}

/// set-mode リクエストを処理し、correlation 付きで応答する
/// — この FSW の business logic。
///
/// request-response では、受理でも拒否でも **必ず ack を返す**。
/// ack は payload に元リクエストの `req-id` を echo するので、地上は
/// 「どのコマンドの結果か」を対応付けられる。
///
/// 運用判断の余地: 有効モード集合、遷移可否ルール、reject 理由コードの
/// 詳細度、req-id 欠落時の扱い。ここでは最小形に留めている。
fn handle_set_mode(current: &mut String, payload: &Payload) {
    // correlation id。欠落していても動く（その場合は -1 を echo）。
    let req_id = match msg::get(payload, "req-id") {
        Some(Value::Integer(id)) => *id,
        _ => -1,
    };
    let target = msg::get_text(payload, "mode").unwrap_or("");

    let (status, applied): (&str, String) = if VALID_MODES.contains(&target) {
        *current = target.to_string();
        ("accepted", current.clone())
    } else {
        // 拒否: モードは変えず、現在モードを返す。
        ("rejected", current.clone())
    };

    msg::send_to(
        NodeId::Ground,
        KIND_SET_MODE_ACK,
        msg::key_value([
            ("req-id", Value::Integer(req_id)),
            ("status", Value::Text(status.to_string())),
            ("mode", Value::Text(applied)),
        ]),
    );
}

orts_plugin!(Controller, mode);
