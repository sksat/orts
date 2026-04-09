//! orts plugin SDK — WASM Component ゲスト開発用。
//!
//! 2 つの書き方を提供する:
//!
//! ## コールバック型 (`orts_plugin!`)
//!
//! ホストが毎 tick `update()` を呼ぶ。WIT world: `plugin`。
//!
//! ```ignore
//! #[allow(warnings)]
//! mod bindings;
//! use bindings::orts::plugin::types::*;
//!
//! struct MyController { /* ... */ }
//!
//! impl MyController {
//!     fn sample_period(&self) -> f64 { 1.0 }
//!     fn init(config: &str) -> Result<Self, String> { Ok(Self { }) }
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

/// コールバック型プラグインのボイラープレートを生成する。
///
/// `$ty` は以下のメソッドを持つ必要がある:
///
/// - `fn sample_period(&self) -> f64`
/// - `fn init(config: &str) -> Result<Self, String>`
/// - `fn update(&mut self, input: &TickInput) -> Result<Option<Command>, String>`
///
/// オプション（`mode` 指定時に必要）:
///
/// - `fn current_mode(&self) -> Option<&str>`
///
/// # 使い方
///
/// ```ignore
/// orts_plugin!(MyController);           // current_mode は常に None
/// orts_plugin!(MyController, mode);     // current_mode を $ty に委譲
/// ```
#[macro_export]
macro_rules! orts_plugin {
    // current_mode を委譲するバリアント
    ($ty:ty, mode) => {
        $crate::__orts_plugin_impl!($ty, {
            __ORTS_PLUGIN_STATE.with(|s| {
                s.borrow().as_ref().expect("plugin not initialized").current_mode().map(::std::string::String::from)
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
                    s.borrow()
                        .as_ref()
                        .expect("plugin not initialized")
                        .sample_period()
                })
            }

            fn init(config: ::std::string::String) -> ::core::result::Result<(), ::std::string::String> {
                let instance = <$ty>::init(&config)?;
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
                    s.borrow_mut()
                        .as_mut()
                        .expect("plugin not initialized")
                        .update(&input)
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
