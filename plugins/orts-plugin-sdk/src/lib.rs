//! orts plugin SDK — WASM Component ゲスト開発用。
//!
//! 2 つの書き方を提供する:
//!
//! ## コールバック型 (`Plugin` trait + `orts_plugin!`)
//!
//! ホストが毎 tick `update()` を呼ぶ。WIT world: `plugin`。
//!
//! ```ignore
//! #[allow(warnings)]
//! mod bindings;
//! use bindings::orts::plugin::types::*;
//! use orts_plugin_sdk::{Plugin, orts_plugin};
//!
//! struct MyController { sample_period: f64 }
//!
//! impl Plugin<TickInput, Command> for MyController {
//!     fn sample_period(&self) -> f64 { self.sample_period }
//!     fn init(config: &str) -> Result<Self, String> {
//!         Ok(Self { sample_period: 1.0 })
//!     }
//!     fn update(&mut self, input: &TickInput) -> Result<Option<Command>, String> {
//!         Ok(None)
//!     }
//! }
//!
//! orts_plugin!(MyController);
//! ```
//!
//! ## メインループ型
//!
//! ゲストが制御フローを持つ。WIT world: `plugin-loop`。
//! SDK を使わず WIT bindgen の関数を直接呼んでも動く。
//!
//! ```ignore
//! fn main() {
//!     loop {
//!         let input = wait_tick();
//!         send_command(my_command);
//!     }
//! }
//! ```

pub mod mode;

/// コールバック型プラグインの trait。
///
/// 型パラメータ `I` は入力型（WIT の `TickInput`）、`C` はコマンド型（WIT の `Command`）。
/// WIT bindgen が生成する型が crate ごとに異なるためジェネリクスにしている。
pub trait Plugin<I, C> {
    /// ホストが `update` を呼ぶ固定サンプル周期 [s]。
    fn sample_period(&self) -> f64;

    /// 設定文字列からインスタンスを生成する。空文字列ならデフォルト。
    fn init(config: &str) -> Result<Self, String>
    where
        Self: Sized;

    /// 1 tick 分の制御計算。`None` はコマンドなし（前回値を ZOH 保持）。
    fn update(&mut self, input: &I) -> Result<Option<C>, String>;

    /// 現在のモード名。モード遷移を持たない場合は `None`。
    fn current_mode(&self) -> Option<&str> {
        None
    }
}

/// コールバック型プラグインのボイラープレートを生成する。
///
/// `$ty` は `Plugin<TickInput, Command>` を実装している必要がある。
///
/// # 使い方
///
/// ```ignore
/// orts_plugin!(MyController);           // current_mode は常に None
/// orts_plugin!(MyController, mode);     // current_mode を Plugin::current_mode に委譲
/// ```
#[macro_export]
macro_rules! orts_plugin {
    // current_mode を委譲するバリアント
    ($ty:ty, mode) => {
        $crate::__orts_plugin_impl!($ty, {
            __ORTS_PLUGIN_STATE.with(|s| {
                $crate::Plugin::current_mode(
                    s.borrow().as_ref().expect("plugin not initialized")
                ).map(::std::string::String::from)
            })
        });
    };
    // current_mode は常に None
    ($ty:ty) => {
        $crate::__orts_plugin_impl!($ty, { None });
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __orts_plugin_impl {
    ($ty:ty, { $current_mode_body:expr }) => {
        struct __OrtsPluginComponent;

        impl bindings::exports::orts::plugin::controller::Guest for __OrtsPluginComponent {
            fn sample_period_s() -> f64 {
                __ORTS_PLUGIN_STATE.with(|s| {
                    $crate::Plugin::sample_period(
                        s.borrow().as_ref().expect("plugin not initialized")
                    )
                })
            }

            fn init(config: ::std::string::String) -> ::core::result::Result<(), ::std::string::String> {
                let instance = <$ty as $crate::Plugin<
                    bindings::orts::plugin::types::TickInput,
                    bindings::orts::plugin::types::Command,
                >>::init(&config)?;
                __ORTS_PLUGIN_STATE.with(|s| *s.borrow_mut() = Some(instance));
                Ok(())
            }

            fn update(
                input: bindings::orts::plugin::types::TickInput,
            ) -> ::core::result::Result<
                ::core::option::Option<bindings::orts::plugin::types::Command>,
                ::std::string::String,
            > {
                __ORTS_PLUGIN_STATE.with(|s| {
                    $crate::Plugin::update(
                        s.borrow_mut().as_mut().expect("plugin not initialized"),
                        &input,
                    )
                })
            }

            fn current_mode() -> ::core::option::Option<::std::string::String> {
                $current_mode_body
            }
        }

        ::std::thread_local! {
            static __ORTS_PLUGIN_STATE: ::core::cell::RefCell<::core::option::Option<$ty>> =
                const { ::core::cell::RefCell::new(None) };
        }

        bindings::export!(__OrtsPluginComponent with_types_in bindings);
    };
}
