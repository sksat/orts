//! モード遷移機構。
//!
//! コールバック型プラグインで複数モードを扱うためのオプション。
//! メインループ型では不要（普通の if/match/break で書ける）。
//!
//! # 使い方
//!
//! ```ignore
//! use orts_plugin_sdk::mode::{Mode, ModeOutput, ModeRunner};
//! use bindings::orts::plugin::types::*;
//!
//! struct Detumble { gain: f64 }
//!
//! impl Mode<TickInput, Command> for Detumble {
//!     fn name(&self) -> &'static str { "detumble" }
//!     fn update(&mut self, input: &TickInput) -> ModeOutput<TickInput, Command> {
//!         // 収束したら遷移
//!         if converged(input) {
//!             ModeOutput::transition(None, NadirPoint::new())
//!         } else {
//!             ModeOutput::command(Some(bdot_command(input)))
//!         }
//!     }
//! }
//!
//! // ModeRunner を orts_plugin! と組み合わせる:
//! struct MyController {
//!     runner: ModeRunner<TickInput, Command>,
//! }
//!
//! impl MyController {
//!     fn sample_period(&self) -> f64 { 1.0 }
//!     fn init(config: &str) -> Result<Self, String> {
//!         Ok(Self { runner: ModeRunner::new(Detumble { gain: 1e4 }) })
//!     }
//!     fn update(&mut self, input: &TickInput) -> Result<Option<Command>, String> {
//!         Ok(self.runner.update(input))
//!     }
//!     fn current_mode(&self) -> Option<&str> {
//!         Some(self.runner.current_mode_name())
//!     }
//! }
//!
//! orts_plugin!(MyController, mode);
//! ```

use alloc::boxed::Box;

/// 1 tick の実行結果。
pub struct ModeOutput<I, C> {
    /// このモードが出すコマンド。
    pub command: Option<C>,
    /// 次のモードに遷移する場合 `Some`。`None` なら現在のモードを維持。
    pub next: Option<Box<dyn Mode<I, C>>>,
}

impl<I, C> ModeOutput<I, C> {
    /// コマンドのみ。モード遷移なし。
    pub fn command(cmd: Option<C>) -> Self {
        Self {
            command: cmd,
            next: None,
        }
    }

    /// コマンド + モード遷移。
    pub fn transition(cmd: Option<C>, next: impl Mode<I, C> + 'static) -> Self {
        Self {
            command: cmd,
            next: Some(Box::new(next)),
        }
    }
}

/// 制御モード。
///
/// 型パラメータ `I` は入力型（WIT の `TickInput`）、`C` はコマンド型（WIT の `Command`）。
/// ジェネリクスにしているのは、WIT bindgen が生成する型が crate ごとに異なるため。
pub trait Mode<I, C> {
    /// モード名。ホスト側で `current_mode()` として公開される。
    fn name(&self) -> &'static str;

    /// 1 tick の更新。
    fn update(&mut self, input: &I) -> ModeOutput<I, C>;
}

/// モード遷移を管理するランナー。
pub struct ModeRunner<I, C> {
    current: Box<dyn Mode<I, C>>,
}

impl<I, C> ModeRunner<I, C> {
    pub fn new(initial: impl Mode<I, C> + 'static) -> Self {
        Self {
            current: Box::new(initial),
        }
    }

    /// 現在のモードを 1 tick 更新し、コマンドを返す。
    /// モード遷移が発生した場合は自動的に切り替える。
    pub fn update(&mut self, input: &I) -> Option<C> {
        let output = self.current.update(input);
        if let Some(next) = output.next {
            self.current = next;
        }
        output.command
    }

    /// 現在のモード名。
    pub fn current_mode_name(&self) -> &str {
        self.current.name()
    }
}
