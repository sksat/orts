//! ノード間メッセージング（運用 C&T + 将来の ISL）のホスト側型。
//!
//! WIT `msg-io` interface の transport 層に対応する host-native 型。
//! `tick-io` の [`Command`](super::Command)（FSW→アクチュエータの制御出力）
//! とは **別物**で、運用者(地上)↔FSW のコマンド/テレメトリ、および将来の
//! 衛星間通信(ISL)を運ぶ。
//!
//! ## レイヤリング
//!
//! transport は **dumb pipe** であり payload の意味を解釈しない。
//! request-response / fire-and-forget / ack / file-transfer といった
//! interaction model は **アプリ層**（[`Payload`] の中身 + SDK ヘルパ）に
//! 属する。`kind` は content-type / 次プロトコル識別子で、受信側がこれを
//! 見て payload の解釈プロトコルを選ぶ。
//!
//! WIT 生成型はホスト側 wasm モジュール内に閉じている（sync / async で
//! 別型になる）。ここで host-native 型を定義し、
//! [`super::wasm::convert`] が WIT 型との相互変換を担う — 既存の
//! [`Command`](super::Command) / [`TickInput`](super::TickInput) と同じ構造。

/// ノードアドレス。運用者 C&T = [`NodeId::Ground`] ↔ [`NodeId::Satellite`]、
/// ISL = `Satellite` ↔ `Satellite`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeId {
    /// 地上局 / 運用者。
    Ground,
    /// 衛星（ID）。
    Satellite(u32),
}

/// 型付きスカラ値（[`Payload::KeyValue`] の要素値）。
///
/// `kind` が論理型を識別する一方、`key-value` の各値はこの variant で
/// 型付けされる。`binary` / `json` payload では型付けは受信側プロトコルの
/// 責務（transport は中身を解釈しない）。
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// 真偽値。
    Boolean(bool),
    /// 符号付き 64bit 整数。
    Integer(i64),
    /// 倍精度浮動小数点。
    Number(f64),
    /// UTF-8 文字列。
    Text(String),
    /// 生バイト列。
    Bytes(Vec<u8>),
}

impl Value {
    /// `Boolean` なら中身を返す。
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// `Integer` なら中身を返す。
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    /// `Number` なら中身を返す。
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// `Text` なら中身を返す。
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    /// `Bytes` なら中身を返す。
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }
}

/// 名前付き値（key-value の 1 要素）。
#[derive(Debug, Clone, PartialEq)]
pub struct NamedValue {
    /// キー名。
    pub name: String,
    /// 値。
    pub value: Value,
}

/// メッセージ本体のエンコーディング。ユースケースで選択する。
///
/// 同一 `kind` では canonical encoding を 1 つに固定する規約
/// （json / key-value / binary を同じ `kind` で混在させない）。
#[derive(Debug, Clone, PartialEq)]
pub enum Payload {
    /// 構造化（スカラは型付き）。viewer/CLI/test から扱いやすい。
    KeyValue(Vec<NamedValue>),
    /// 生バイナリ（CCSDS 等、デコードは受信側 FSW）。
    Binary(Vec<u8>),
    /// JSON テキスト。transport は opaque 文字列として扱う。
    Json(String),
}

impl Payload {
    /// `(name, value)` のイテレータから [`Payload::KeyValue`] を構築する。
    ///
    /// 並び順は保持される（決定論的）。
    pub fn key_value<I, S>(pairs: I) -> Self
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

    /// key-value から名前で値を引く。
    ///
    /// `KeyValue` 以外、または該当キーがなければ `None`。同名キーが複数
    /// ある場合は最初の 1 つを返す。
    pub fn get(&self, name: &str) -> Option<&Value> {
        match self {
            Payload::KeyValue(kvs) => kvs.iter().find(|kv| kv.name == name).map(|kv| &kv.value),
            _ => None,
        }
    }
}

/// 受信側へ届く完全なメッセージ。
///
/// `src` / `host_seq` / `deliver_tick` はホストが確定して埋める
/// （ゲストは送信時にこれらを指定できない — なりすまし防止と決定論）。
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    /// 送信元。ホストが注入。
    pub src: NodeId,
    /// 宛先。
    pub dst: NodeId,
    /// 論理型（content-type）。例 `"orts.cmd.set-mode.v1"`。
    pub kind: String,
    /// ホストが割り当てる決定論的な全順序の anchor。
    pub host_seq: u64,
    /// ホストが配送を確定した tick。
    pub deliver_tick: u64,
    /// 本体。
    pub payload: Payload,
}

/// ゲスト（FSW）が送信するメッセージ。最小限。
///
/// `src` / `host_seq` / `deliver_tick` はホストが補完して [`Message`] になる。
#[derive(Debug, Clone, PartialEq)]
pub struct Outbound {
    /// 宛先（地上なら [`NodeId::Ground`]、別衛星なら `Satellite(id)`）。
    pub dst: NodeId,
    /// 論理型（content-type）。
    pub kind: String,
    /// 本体。
    pub payload: Payload,
}

impl Outbound {
    /// 宛先 + kind + payload から構築する。
    pub fn new(dst: NodeId, kind: impl Into<String>, payload: Payload) -> Self {
        Self {
            dst,
            kind: kind.into(),
            payload,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_value_preserves_order_and_lookup() {
        let p = Payload::key_value([
            ("mode", Value::Text("nadir".into())),
            ("priority", Value::Integer(3)),
        ]);
        // 並び順保持
        if let Payload::KeyValue(kvs) = &p {
            assert_eq!(kvs[0].name, "mode");
            assert_eq!(kvs[1].name, "priority");
        } else {
            panic!("expected KeyValue");
        }
        // 名前引き
        assert_eq!(p.get("mode").and_then(Value::as_text), Some("nadir"));
        assert_eq!(p.get("priority").and_then(Value::as_integer), Some(3));
        assert!(p.get("missing").is_none());
    }

    #[test]
    fn get_on_non_key_value_is_none() {
        assert!(Payload::Binary(vec![1, 2, 3]).get("x").is_none());
        assert!(Payload::Json("{}".into()).get("x").is_none());
    }

    #[test]
    fn value_accessors_are_typed() {
        assert_eq!(Value::Boolean(true).as_bool(), Some(true));
        assert_eq!(Value::Number(1.5).as_number(), Some(1.5));
        assert_eq!(Value::Text("hi".into()).as_text(), Some("hi"));
        assert_eq!(Value::Bytes(vec![0xAB]).as_bytes(), Some(&[0xAB][..]));
        // 型違いは None
        assert_eq!(Value::Integer(7).as_number(), None);
    }
}
