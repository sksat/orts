//! orts plugin SDK — WASM Component ゲスト開発用。
//!
//! ホストは **ループベース** (`plugin` world) でゲストを駆動する。
//! ゲストは 2 つの書き方を選べる:
//!
//! ## 1. コールバック型（`Plugin` trait + `orts_plugin!` マクロ）
//!
//! `Plugin<TickInput, Command>` を実装すると、SDK マクロがメインループを
//! 自動生成して plugin world の `run()` export にする。ユーザーは毎 tick の
//! 制御計算だけ書けばよい。
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
//! ## 2. メインループ型（`wait-tick` / `send-command` を直接使う）
//!
//! シーケンシャルな制御（Phase 1 → wait → Phase 2）を素直に書きたい場合。
//! SDK マクロを使わず `impl Guest for Component` を手書きする。
//!
//! ```ignore
//! use bindings::orts::plugin::tick_io::{send_command, wait_tick};
//!
//! impl bindings::Guest for Component {
//!     fn metadata() -> PluginMetadata { PluginMetadata { sample_period_s: 1.0 } }
//!     fn current_mode() -> Option<String> { None }
//!     fn run(config: String) -> Result<(), String> {
//!         loop {
//!             let input = wait_tick();
//!             send_command(my_command(&input));
//!         }
//!     }
//! }
//! ```

pub mod mode;

/// コールバック型プラグインの trait。
///
/// 型パラメータ `I` は入力型（WIT の `TickInput`）、`C` はコマンド型（WIT の `Command`）。
/// WIT bindgen が生成する型が crate ごとに異なるためジェネリクスにしている。
pub trait Plugin<I, C> {
    /// 希望サンプル周期 \[s\]。ホストはこれを参考に tick 間隔を決める。
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
/// マクロは plugin world の `metadata` / `current-mode` / `run` export を
/// 生成し、`run()` の中で `wait_tick()` → `update()` → `send_command()` の
/// ループを回す。
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

        impl bindings::Guest for __OrtsPluginComponent {
            fn metadata(
                config: ::std::string::String,
            ) -> ::core::result::Result<
                bindings::orts::plugin::types::PluginMetadata,
                ::std::string::String,
            > {
                // config の早期検証を兼ねて一度 init を呼ぶ。ここで作った
                // インスタンスは捨て、run 側で改めて初期化する。
                let instance = <$ty as $crate::Plugin<
                    bindings::orts::plugin::types::TickInput,
                    bindings::orts::plugin::types::Command,
                >>::init(&config)?;
                let sample_period = $crate::Plugin::sample_period(&instance);
                if !sample_period.is_finite() || sample_period <= 0.0 {
                    return ::core::result::Result::Err(::std::format!(
                        "invalid sample_period: {}",
                        sample_period
                    ));
                }
                ::core::result::Result::Ok(bindings::orts::plugin::types::PluginMetadata {
                    sample_period_s: sample_period,
                })
            }

            fn current_mode() -> ::core::option::Option<::std::string::String> {
                $current_mode_body
            }

            fn run(config: ::std::string::String) -> ::core::result::Result<(), ::std::string::String> {
                let instance = <$ty as $crate::Plugin<
                    bindings::orts::plugin::types::TickInput,
                    bindings::orts::plugin::types::Command,
                >>::init(&config)?;
                __ORTS_PLUGIN_STATE.with(|s| *s.borrow_mut() = Some(instance));

                loop {
                    let input = match bindings::orts::plugin::tick_io::wait_tick() {
                        ::core::option::Option::Some(input) => input,
                        // ホストが shutdown を要求している。正常終了する。
                        ::core::option::Option::None => return ::core::result::Result::Ok(()),
                    };
                    let cmd = __ORTS_PLUGIN_STATE.with(|s| {
                        $crate::Plugin::update(
                            s.borrow_mut().as_mut().expect("plugin not initialized"),
                            &input,
                        )
                    })?;
                    if let ::core::option::Option::Some(cmd) = cmd {
                        bindings::orts::plugin::tick_io::send_command(cmd);
                    }
                }
            }
        }

        ::std::thread_local! {
            static __ORTS_PLUGIN_STATE: ::core::cell::RefCell<::core::option::Option<$ty>> =
                const { ::core::cell::RefCell::new(None) };
        }

        bindings::export!(__OrtsPluginComponent with_types_in bindings);
    };
}
