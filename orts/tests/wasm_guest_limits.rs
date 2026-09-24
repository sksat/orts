//! The limits every WASM controller guest runs under ([`GuestLimits`]): a guest
//! that breaks one fails the call with an error, on both backends, instead of
//! hanging its caller or taking the process's memory.
//!
//! Uses the `misbehaving-guest` fixture in `plugin-sdk/examples`, whose config
//! picks the fault. The tests shorten the turn deadline to [`DEADLINE`] through
//! `WasmPluginCache::with_guest_limits`, so each case takes a fraction of a
//! second rather than the default 5 s. Skips cleanly when the fixture has not
//! been built:
//!
//! ```sh
//! cd plugin-sdk/examples && cargo +1.91.0 component build --release -p orts-test-guest-misbehaving
//! ```

#![cfg(feature = "plugin-wasm-async")]

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use orts::plugin::wasm::{GuestLimits, WasmPluginCache};
use orts::plugin::{PluginController, PluginError, TickInput};

/// The turn deadline these tests run under.
const DEADLINE: Duration = Duration::from_millis(200);

/// How long a case may take before the test calls it hung. Far past anything
/// a case needs (the longest waits one deadline plus the 1 s host-wait grace),
/// so reaching it means a limit did not work.
const HANG: Duration = Duration::from_secs(30);

fn fixture() -> Option<PathBuf> {
    let path = PathBuf::from(format!(
        "{}/../plugin-sdk/examples/target/wasm32-wasip1/release/\
         orts_test_guest_misbehaving.wasm",
        env!("CARGO_MANIFEST_DIR")
    ));
    if path.exists() {
        Some(path)
    } else {
        eprintln!(
            "WASM not found: {}\nBuild: cd plugin-sdk/examples && cargo +1.91.0 component \
             build --release -p orts-test-guest-misbehaving",
            path.display()
        );
        None
    }
}

#[derive(Clone, Copy, Debug)]
enum Backend {
    Sync,
    Async,
}

const BACKENDS: [Backend; 2] = [Backend::Sync, Backend::Async];

fn short_limits() -> GuestLimits {
    GuestLimits {
        turn_deadline: DEADLINE,
        ..GuestLimits::default()
    }
}

/// Build a controller running the fixture with `config`.
fn build(
    cache: &mut WasmPluginCache,
    backend: Backend,
    path: &std::path::Path,
    config: &str,
) -> Result<Box<dyn PluginController>, PluginError> {
    let body = arika::body::KnownBody::Earth;
    Ok(match backend {
        Backend::Sync => Box::new(cache.build_sync_controller(path, "sat", config, body)?),
        Backend::Async => Box::new(cache.build_async_controller(path, "sat", config, body)?),
    })
}

/// Run `f` on a thread of its own and panic if it has not finished within
/// [`HANG`], so a limit that fails to stop a guest fails the test instead of
/// hanging it. A panic inside `f` is passed on as it is.
fn within_hang_bound<T: Send + 'static>(what: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(HANG) {
        Ok(value) => value,
        Err(mpsc::RecvTimeoutError::Disconnected) => match handle.join() {
            Err(panic) => std::panic::resume_unwind(panic),
            Ok(()) => unreachable!("{what} ended without a result"),
        },
        Err(mpsc::RecvTimeoutError::Timeout) => panic!("{what} did not finish within {HANG:?}"),
    }
}

/// A tick with any valid state: the fixture reads nothing from it. Leaked so
/// the input can live on the test's worker thread for as long as it needs.
fn tick_input() -> TickInput<'static> {
    use nalgebra::{Vector3, Vector4};
    use orts::OrbitalState;
    use orts::SpacecraftState;
    use orts::attitude::AttitudeState;
    use orts::plugin::tick_input::{ActuatorTelemetry, Sensors};
    let spacecraft: &'static SpacecraftState = Box::leak(Box::new(SpacecraftState {
        orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
        attitude: AttitudeState {
            quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::new(0.0, 0.0, 0.0),
        },
        mass: 50.0,
    }));
    let sensors: &'static Sensors = Box::leak(Box::new(Sensors {
        magnetometers: vec![],
        gyroscopes: vec![],
        star_trackers: vec![],
        sun_sensors: vec![],
    }));
    let actuators: &'static ActuatorTelemetry = Box::leak(Box::default());
    TickInput {
        t: 0.0,
        epoch: None,
        spacecraft,
        sensors,
        actuators,
    }
}

/// A guest that never returns from `metadata` fails construction once its
/// turn passes the deadline.
#[test]
fn a_guest_spinning_in_metadata_fails_construction() {
    let Some(path) = fixture() else { return };
    for backend in BACKENDS {
        let p = path.clone();
        let (err, elapsed) = within_hang_bound("construction", move || {
            let mut cache = WasmPluginCache::new()
                .unwrap()
                .with_guest_limits(short_limits());
            let started = Instant::now();
            let err = build(&mut cache, backend, &p, r#"{"fault":"spin-in-init"}"#)
                .err()
                .map(|e| e.to_string());
            (err, started.elapsed())
        });
        let err = err.unwrap_or_else(|| panic!("{backend:?}: a spinning metadata is refused"));
        assert!(err.contains("turn deadline"), "{backend:?}: {err}");
        assert!(
            elapsed >= DEADLINE,
            "{backend:?}: stopped after {elapsed:?}"
        );
    }
}

/// A guest that never returns from a tick fails that `update`, and dropping
/// the controller does not wait on it.
#[test]
fn a_guest_spinning_in_update_fails_the_tick() {
    let Some(path) = fixture() else { return };
    for backend in BACKENDS {
        let p = path.clone();
        let (err, dropped_in) = within_hang_bound("update", move || {
            let mut cache = WasmPluginCache::new()
                .unwrap()
                .with_guest_limits(short_limits());
            let mut ctrl = build(&mut cache, backend, &p, r#"{"fault":"spin-in-update"}"#)
                .expect("metadata is well-behaved");
            let err = ctrl.update(&tick_input()).err().map(|e| e.to_string());
            let started = Instant::now();
            drop(ctrl);
            (err, started.elapsed())
        });
        let err = err.unwrap_or_else(|| panic!("{backend:?}: a spinning update fails"));
        assert!(err.contains("turn deadline"), "{backend:?}: {err}");
        assert!(
            dropped_in < Duration::from_secs(5),
            "{backend:?}: drop took {dropped_in:?}"
        );
    }
}

/// Waiting for the next tick is not part of any turn: a controller left idle
/// for longer than the deadline still ticks.
#[test]
fn waiting_for_a_tick_does_not_count_against_the_deadline() {
    let Some(path) = fixture() else { return };
    for backend in BACKENDS {
        let p = path.clone();
        within_hang_bound("idle ticks", move || {
            let mut cache = WasmPluginCache::new()
                .unwrap()
                .with_guest_limits(short_limits());
            let mut ctrl = build(&mut cache, backend, &p, "").expect("a well-behaved guest");
            ctrl.update(&tick_input()).expect("first tick");
            std::thread::sleep(DEADLINE * 3);
            ctrl.update(&tick_input())
                .unwrap_or_else(|e| panic!("{backend:?}: idle time counted: {e}"));
        });
    }
}

/// A guest growing its memory past the limit fails the tick with the limit
/// named, and the process carries on.
#[test]
fn a_guest_growing_memory_fails_with_the_limit() {
    let Some(path) = fixture() else { return };
    let limits = GuestLimits {
        memory_bytes: 16 * 1024 * 1024,
        ..short_limits()
    };
    for backend in BACKENDS {
        let p = path.clone();
        let err = within_hang_bound("memory growth", move || {
            let mut cache = WasmPluginCache::new().unwrap().with_guest_limits(limits);
            let mut ctrl = build(
                &mut cache,
                backend,
                &p,
                r#"{"fault":"grow-memory-in-update"}"#,
            )
            .expect("metadata is well-behaved");
            ctrl.update(&tick_input()).err().map(|e| e.to_string())
        });
        let err = err.unwrap_or_else(|| panic!("{backend:?}: growth past the limit fails"));
        assert!(
            err.contains("linear memory") && err.contains("16777216-byte limit"),
            "{backend:?}: {err}"
        );
    }
}

/// A memory limit below what the guest starts with fails instantiation.
#[test]
fn a_memory_limit_below_the_initial_memory_fails_instantiation() {
    let Some(path) = fixture() else { return };
    let limits = GuestLimits {
        memory_bytes: 64 * 1024,
        ..short_limits()
    };
    for backend in BACKENDS {
        let p = path.clone();
        let err = within_hang_bound("instantiation", move || {
            let mut cache = WasmPluginCache::new().unwrap().with_guest_limits(limits);
            build(&mut cache, backend, &p, "")
                .err()
                .map(|e| e.to_string())
        });
        let err = err.unwrap_or_else(|| panic!("{backend:?}: the guest does not fit"));
        assert!(
            err.contains("instantiate") && err.contains("65536-byte limit"),
            "{backend:?}: {err}"
        );
    }
}

/// `wait-tick` from `metadata` fails construction at once. No tick comes
/// before the constructor returns, so a host that waited would wait forever,
/// and no epoch reaches a guest blocked in a host call.
#[test]
fn wait_tick_before_run_fails_construction_at_once() {
    let Some(path) = fixture() else { return };
    for backend in BACKENDS {
        let p = path.clone();
        let (err, elapsed) = within_hang_bound("construction", move || {
            // The default 5 s deadline: the refusal must not wait for it.
            let mut cache = WasmPluginCache::new().unwrap();
            // Compile the component first, so the time below is instantiation
            // and `metadata` alone. Measured in CI: a debug build compiling it
            // under parallel tests took 5.5 s, past the bound on its own.
            build(&mut cache, backend, &p, "").expect("a well-behaved guest");
            let started = Instant::now();
            let err = build(&mut cache, backend, &p, r#"{"fault":"wait-tick-in-init"}"#)
                .err()
                .map(|e| e.to_string());
            (err, started.elapsed())
        });
        let err = err.unwrap_or_else(|| panic!("{backend:?}: wait-tick in metadata is refused"));
        assert!(err.contains("wait-tick before run"), "{backend:?}: {err}");
        assert!(
            elapsed < Duration::from_secs(4),
            "{backend:?}: refused after {elapsed:?}"
        );
    }
}

/// A guest cannot sleep in a host call: WASI clock waits are ready at once, so
/// an hour's `std::thread::sleep` returns and the tick completes (well inside
/// the 1 s grace after which a stuck guest's tick would fail). A guest
/// blocked there would run no wasm, out of the epoch's reach, and on the sync
/// backend hold an OS thread the host cannot reclaim.
#[test]
fn a_guest_cannot_sleep_in_a_host_call() {
    let Some(path) = fixture() else { return };
    for backend in BACKENDS {
        let p = path.clone();
        let (result, elapsed) = within_hang_bound("update", move || {
            let mut cache = WasmPluginCache::new()
                .unwrap()
                .with_guest_limits(short_limits());
            let mut ctrl = build(&mut cache, backend, &p, r#"{"fault":"sleep-in-update"}"#)
                .expect("metadata is well-behaved");
            let started = Instant::now();
            let result = ctrl
                .update(&tick_input())
                .map(|_| ())
                .map_err(|e| e.to_string());
            (result, started.elapsed())
        });
        result.unwrap_or_else(|e| panic!("{backend:?}: the sleep returned at once: {e}"));
        // Below the host-wait grace: a guest that slept would have had its
        // tick fail as stuck by then, which the `unwrap` above catches too.
        assert!(
            elapsed < Duration::from_secs(1),
            "{backend:?}: the tick took {elapsed:?}, as if the guest slept"
        );
    }
}

/// A guest sending past the per-turn message limit fails the tick.
///
/// Under the default deadline: sending thousands of messages takes longer than
/// [`DEADLINE`] in a debug build (measured: 3000 did not fit in 0.2 s).
#[test]
fn a_message_flood_fails_the_tick() {
    let Some(path) = fixture() else { return };
    for backend in BACKENDS {
        let p = path.clone();
        let err = within_hang_bound("update", move || {
            let mut cache = WasmPluginCache::new().unwrap();
            let mut ctrl = build(
                &mut cache,
                backend,
                &p,
                r#"{"fault":"flood-messages-in-update","count":5000}"#,
            )
            .expect("metadata is well-behaved");
            ctrl.update(&tick_input()).err().map(|e| e.to_string())
        });
        let err = err.unwrap_or_else(|| panic!("{backend:?}: a flood fails the tick"));
        assert!(
            err.contains("msg-io") && err.contains("in one tick"),
            "{backend:?}: {err}"
        );
    }
}

/// Messages a caller never takes stop the simulation once they pass the
/// limit, and a caller that takes them each tick goes on. Under the default
/// deadline, as the flood above.
#[test]
fn messages_nobody_takes_are_bounded() {
    let Some(path) = fixture() else { return };
    let config = r#"{"fault":"flood-messages-in-update","count":3000}"#;
    for backend in BACKENDS {
        let p = path.clone();
        let (untaken, taken) = within_hang_bound("updates", move || {
            let mut cache = WasmPluginCache::new().unwrap();
            let mut ctrl = build(&mut cache, backend, &p, config).expect("well-behaved metadata");
            ctrl.update(&tick_input())
                .expect("3000 messages fit in one tick");
            // 6000 waiting: within the backlog of twice a turn's limit.
            ctrl.update(&tick_input())
                .expect("two ticks of messages fit");
            let untaken = ctrl.update(&tick_input()).err().map(|e| e.to_string());

            let mut ctrl = build(&mut cache, backend, &p, config).expect("well-behaved metadata");
            let mut taken = Ok(());
            for _ in 0..3 {
                taken = taken.and_then(|()| ctrl.update(&tick_input()).map(|_| ()));
                assert_eq!(ctrl.take_outbound().len(), 3000);
            }
            (untaken, taken.map_err(|e| e.to_string()))
        });
        let err = untaken.unwrap_or_else(|| panic!("{backend:?}: 9000 untaken messages fail"));
        assert!(err.contains("backlog overrun"), "{backend:?}: {err}");
        taken.unwrap_or_else(|e| panic!("{backend:?}: a draining caller failed: {e}"));
    }
}
