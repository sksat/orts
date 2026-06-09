//! ノード間メッセージング（運用 C&T + 将来の ISL）のゲスト側ヘルパ。
//!
//! WIT `msg-io` interface への薄いラッパ + key-value [`Payload`] の構築/
//! 読み出しヘルパ。`tick-io` の制御ループ（[`Plugin::update`](crate::Plugin)
//! / `send-command`）とは独立して、`update` / `run` の tick 内から呼べる。
//!
//! ## レイヤリング
//!
//! transport（`msg-io`）は dumb pipe で、payload の意味を解釈しない。
//! fire-and-forget / request-response / ack といった interaction model は
//! **このアプリ層で組み立てる**（`kind` の規約 + payload の中身）。
//! 同じ transport の上で両モデルを実装できる:
//!
//! - **fire-and-forget**: コマンドを送って応答を期待しない。確認は別途
//!   流れるテレメトリを観測して行う（`examples/commandable-mode-ff`）。
//! - **request-response**: payload に correlation id を入れ、応答が echo
//!   する（`examples/commandable-mode-rr`）。
//!
//! どちらも `kind` は名前空間 + version 規約に従う
//! （例 `"orts.cmd.set-mode.v1"` / `"orts.tlm.mode.v1"`）。

use alloc::string::String;
use alloc::vec::Vec;

use crate::bindings::orts::plugin::msg_io;
pub use crate::bindings::orts::plugin::types::{
    Message, NamedValue, NodeId, Outbound, Payload, Value,
};

/// 現在 tick の凍結受信箱から最大 `max` 件取り出す。
///
/// 返り値が空なら、この tick の受信箱は空になった。ホストは tick 境界で
/// 受信箱を凍結するので、いつ・何回呼んでも観測は決定論的。
pub fn recv_batch(max: u32) -> Vec<Message> {
    msg_io::recv_batch(max)
}

/// 現在 tick の受信箱を全部 drain する。
///
/// 内部で [`recv_batch`] を空になるまで繰り返す。1 tick に大量受信が
/// 想定される場合（ファイル転送等）は、代わりに [`recv_batch`] で件数を
/// 絞ってペース制御すること。
pub fn recv_all() -> Vec<Message> {
    let mut all = Vec::new();
    loop {
        let batch = msg_io::recv_batch(64);
        if batch.is_empty() {
            break;
        }
        all.extend(batch);
    }
    all
}

/// メッセージを送信する（append 意味論 — 1 tick 内の複数送信は全て届く）。
pub fn send(msg: &Outbound) {
    msg_io::send_message(msg);
}

/// 宛先 + kind + payload から送信する簡易ヘルパ。
pub fn send_to(dst: NodeId, kind: impl Into<String>, payload: Payload) {
    msg_io::send_message(&Outbound {
        dst,
        kind: kind.into(),
        payload,
    });
}

/// `(name, value)` のイテレータから key-value [`Payload`] を構築する。
/// 並び順は保持される（決定論的）。
pub fn key_value<I, S>(pairs: I) -> Payload
where
    I: IntoIterator<Item = (S, Value)>,
    S: Into<String>,
{
    Payload::KeyValue(
        pairs
            .into_iter()
            .map(|(name, value)| NamedValue {
                name: name.into(),
                value,
            })
            .collect(),
    )
}

/// key-value [`Payload`] から名前で値を引く。
///
/// `KeyValue` 以外、または該当キーがなければ `None`。
pub fn get<'a>(payload: &'a Payload, name: &str) -> Option<&'a Value> {
    match payload {
        Payload::KeyValue(kvs) => kvs.iter().find(|kv| kv.name == name).map(|kv| &kv.value),
        _ => None,
    }
}

/// key-value [`Payload`] から名前で文字列値を引く近道。
pub fn get_text<'a>(payload: &'a Payload, name: &str) -> Option<&'a str> {
    match get(payload, name) {
        Some(Value::Text(s)) => Some(s),
        _ => None,
    }
}
