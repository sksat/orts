//! バイトストリーム接続（[`kble`](https://github.com/arkedge/kble) 統合 /
//! 将来の RF seam）のゲスト側ヘルパ。
//!
//! WIT `stream-io` interface への薄いラッパ。`msg-io`（packet ベースの運用
//! C&T 模擬）とは別物の兄弟で、FSW が**自前のフレーミング**（EB90 / C2A /
//! CCSDS / 独自）で **生バイト**を喋るための口。orts は中身を解釈しない
//! dumb byte conduit なので、framing/デフレーミングは FSW（このアプリ層）が
//! 担う。
//!
//! ## 配送意味論（決定論）
//!
//! ホストは tick 境界で受信バッファを凍結し送信を flush する。[`read`] は
//! その tick に届いているバイトだけを返し、足りないフレームは次 tick へ
//! 持ち越して再組立する（連続 UART の inter-byte timing は模擬しない）。
//!
//! - [`StreamRead::NoData`]: この tick はデータ無し（相手は生存）。
//! - [`StreamRead::Closed`]: 相手が切断（`NoData` と区別できる）。
//! - `Err(`[`StreamError::Overrun`]`)`: 有界キューが溢れた。ホストは
//!   シミュレーションを停止する（byte drop はしない）。

use alloc::vec::Vec;

use crate::bindings::orts::plugin::stream_io;
pub use crate::bindings::orts::plugin::types::{StreamError, StreamRead};

/// 名前付き stream の凍結受信バッファから最大 `max` バイト取り出す。
///
/// `max == 0` は no-op（[`StreamRead::NoData`]）。`Data` は必ず長さ > 0。
pub fn read(name: &str, max: u32) -> Result<StreamRead, StreamError> {
    stream_io::read(name, max)
}

/// 名前付き stream へバイト列を書く（append、次の tick 境界で flush）。
pub fn write(name: &str, bytes: &[u8]) -> Result<(), StreamError> {
    stream_io::write(name, bytes)
}

/// [`read`] の薄い便利版: 読めたバイトを `Some`、データ無しを `None` で返す。
/// `Closed` も `None`（末尾扱い）になるので、切断を区別したい場合は [`read`]
/// を直接使うこと。
pub fn read_bytes(name: &str, max: u32) -> Result<Option<Vec<u8>>, StreamError> {
    match read(name, max)? {
        StreamRead::Data(bytes) => Ok(Some(bytes)),
        StreamRead::NoData | StreamRead::Closed => Ok(None),
    }
}
