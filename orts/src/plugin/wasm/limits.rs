//! Bounds on what one WASM controller guest may use.
//!
//! A guest is code the host did not write, and since `orts serve` can take
//! components from WebSocket clients, code the host did not choose either. Every
//! guest therefore runs under the same limits, whichever way its bytes arrived:
//!
//! - **Turn deadline** (wall clock). A *turn* is the time the guest holds
//!   control: from the host handing it over — instantiation, the `metadata`
//!   call, the start of `run`, or `wait-tick` returning a tick — until the guest
//!   hands it back by calling `wait-tick` or returning. A turn longer than the
//!   deadline traps. Waiting inside `wait-tick` for the next tick is not part of
//!   any turn. Measured with epoch interruption: [`WasmEngine`] advances the
//!   epoch every [`EPOCH_TICK`], and each store's deadline callback compares the
//!   turn's age against the deadline.
//! - **Memory**: each linear memory and each table is capped, and so is the
//!   number of instances, memories and tables a store may create. Growth past a
//!   cap traps with an error instead of taking host memory.
//! - **Host waits**: a guest can also stop handing control back by blocking in
//!   a host call, where no wasm runs and the epoch cannot reach it. With the
//!   WASI context the guests get (no preopens, stdin closed, every socket
//!   address denied), the wait found is on a clock (`std::thread::sleep`
//!   polls a monotonic-clock subscription), and [`clock_waits_return_at_once`]
//!   makes those subscriptions ready at once. Should another host call block,
//!   the controller stops waiting after the deadline plus [`HOST_WAIT_GRACE`]
//!   and reports the guest as stuck.
//!
//! The deadline is wall time, so whether a guest near it finishes can depend on
//! how loaded the machine is. A run that completes inside the limits produces
//! the same output as before, and the sync and async backends stay bit-exact;
//! a run the deadline stops is incomplete, not different.
//!
//! [`WasmEngine`]: super::WasmEngine

use std::time::{Duration, Instant};

use wasmtime::component::{Linker, Resource};
use wasmtime::{ResourceLimiter, StoreContextMut, UpdateDeadline};
use wasmtime_wasi::p2::DynPollable;

use crate::plugin::PluginError;

/// How often [`super::WasmEngine`] advances the engine epoch.
///
/// A guest past its deadline traps at most this long after the deadline, and
/// each epoch costs one deadline-callback call while a guest runs, which is
/// cheap next to 10 ms of interpreted wasm.
pub const EPOCH_TICK: Duration = Duration::from_millis(10);

/// The default [`GuestLimits::turn_deadline`].
///
/// The example guests take microseconds per turn: `pd-rw-control` updates in
/// about 14 µs in release on Pulley (see `async_controller.rs`). 5 s is five
/// orders of magnitude above that, and still short enough that a guest that
/// never returns holds up `orts serve`'s manager for seconds rather than for
/// good.
pub const DEFAULT_TURN_DEADLINE: Duration = Duration::from_secs(5);

/// How much longer than the turn deadline a controller waits for its guest
/// before calling it stuck in a host call.
///
/// A guest running wasm traps within [`EPOCH_TICK`] of its deadline, so this
/// wait only runs out for a guest that is not running wasm at all.
pub const HOST_WAIT_GRACE: Duration = Duration::from_secs(1);

/// The default [`GuestLimits::memory_bytes`]: 256 MiB per linear memory.
///
/// Measured with a recording limiter: the Rust example guests (bdot,
/// pd-rw-control, commandable-mode, stream-framed-commander, over the orts
/// plugin test suites) never grow past their initial 1,114,112 bytes (17
/// pages), and the C-based `nos3-adcs` debug build reaches 1,179,648 bytes in a
/// 600 s Sun-Safe run. The limit leaves two orders of magnitude over that.
pub const DEFAULT_MEMORY_BYTES: usize = 256 * 1024 * 1024;

/// The default [`GuestLimits::table_elements`] per table.
///
/// Measured as above: the largest table the Rust example guests ask for holds
/// 92 elements, and `nos3-adcs`'s 109 (their indirect-call targets).
pub const DEFAULT_TABLE_ELEMENTS: usize = 65_536;

/// The default [`GuestLimits::instances`], [`GuestLimits::memories`] and
/// [`GuestLimits::tables`]: how many of each one store may create.
///
/// A component built by `cargo component` instantiates a handful of core
/// instances (the guest, the WASI adapter, and the shims between them). The
/// wasmtime default is 10,000 of each, which would let a component multiply its
/// memory limit by that.
pub const DEFAULT_INSTANCES: usize = 32;
/// See [`DEFAULT_INSTANCES`].
pub const DEFAULT_MEMORIES: usize = 4;
/// See [`DEFAULT_INSTANCES`].
pub const DEFAULT_TABLES: usize = 16;

/// The default [`GuestLimits::resources`]: how many component resources (WASI
/// streams, pollables, …) a guest may hold at once.
///
/// A guest built with `cargo component` holds a handful (its stdio streams, a
/// pollable while it polls one). wasmtime's own default is 1,000,000 entries,
/// which a guest keeping every clock subscription it makes would turn into
/// host memory no other limit counts.
pub const DEFAULT_RESOURCES: usize = 1024;

/// The most `host-env.log` records a guest may write in one turn. Past it the
/// turn's further records are dropped, with one warning that says so.
pub const MAX_LOG_RECORDS: usize = 64;

/// The longest `host-env.log` record written; a longer one is cut here.
pub const MAX_LOG_RECORD_BYTES: usize = 4096;

/// The most msg-io messages a guest may send in one turn. A controller keeps
/// twice this for its caller to take (see [`OutboundBacklog`]).
///
/// `send-message` copies the message into host memory, which no linear-memory
/// limit covers: without this, a guest could send the same buffer until the
/// host runs out.
pub const MAX_MESSAGES: usize = 4096;

/// The most msg-io message bytes (kinds, names, text and byte payloads) a guest
/// may send in one turn. A controller keeps twice this for its caller.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

/// Bounds on one guest. [`Default`] gives the constants above.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuestLimits {
    /// Longest the guest may hold control in one turn (see the module docs).
    pub turn_deadline: Duration,
    /// Largest size of each linear memory, in bytes.
    pub memory_bytes: usize,
    /// Largest number of elements in each table.
    pub table_elements: usize,
    /// Most core instances one store may create.
    pub instances: usize,
    /// Most linear memories one store may create.
    pub memories: usize,
    /// Most tables one store may create.
    pub tables: usize,
    /// Most component resources the guest may hold at once.
    pub resources: usize,
}

impl Default for GuestLimits {
    fn default() -> Self {
        Self {
            turn_deadline: DEFAULT_TURN_DEADLINE,
            memory_bytes: DEFAULT_MEMORY_BYTES,
            table_elements: DEFAULT_TABLE_ELEMENTS,
            instances: DEFAULT_INSTANCES,
            memories: DEFAULT_MEMORIES,
            tables: DEFAULT_TABLES,
            resources: DEFAULT_RESOURCES,
        }
    }
}

impl GuestLimits {
    /// How long a controller waits for its guest to hand control back before
    /// calling it stuck: `turns` deadlines, then [`HOST_WAIT_GRACE`]. Saturates,
    /// so a deadline of `Duration::MAX` waits for good instead of overflowing.
    pub(super) fn host_wait(&self, turns: u32) -> Duration {
        self.turn_deadline
            .saturating_mul(turns)
            .saturating_add(HOST_WAIT_GRACE)
    }
}

/// The clock of the turn the guest is in, kept in each store's host state.
#[derive(Debug)]
pub(super) struct TurnClock {
    deadline: Duration,
    started: Instant,
}

impl TurnClock {
    pub(super) fn new(deadline: Duration) -> Self {
        Self {
            deadline,
            started: Instant::now(),
        }
    }

    /// The host hands control to the guest: a new turn starts now.
    pub(super) fn start(&mut self) {
        self.started = Instant::now();
    }

    /// Answer the epoch-deadline callback: trap once the turn has run past
    /// the deadline, else wait for the next epoch. `yield_first` makes an
    /// async store yield to its executor before continuing, so a long turn
    /// does not hold the runtime's worker thread by itself.
    pub(super) fn on_epoch(&self, yield_first: bool) -> wasmtime::Result<UpdateDeadline> {
        let elapsed = self.started.elapsed();
        if elapsed > self.deadline {
            return Err(wasmtime::format_err!(
                "{}",
                turn_deadline_message(self.deadline)
            ));
        }
        #[cfg(feature = "plugin-wasm-async")]
        if yield_first {
            return Ok(UpdateDeadline::YieldCustom(
                1,
                Box::pin(tokio::task::yield_now()),
            ));
        }
        let _ = yield_first;
        Ok(UpdateDeadline::Continue(1))
    }
}

/// What a guest that ran past its turn deadline is told.
pub(super) fn turn_deadline_message(deadline: Duration) -> String {
    format!(
        "the guest held control for more than {} s without returning to the host \
         (turn deadline)",
        deadline.as_secs_f64()
    )
}

/// What the controller reports when its guest did not hand control back in
/// time and was not running wasm either.
pub(super) fn stuck_in_host_message(waited: Duration) -> String {
    format!(
        "the guest did not return to the host within {} s, blocked in a host call \
         the turn deadline cannot interrupt",
        waited.as_secs_f64()
    )
}

/// The WASI monotonic clock's interface, under the name `wasmtime-wasi`
/// defines it in a linker. A guest importing an older 0.2 version is linked to
/// this one by semver.
const MONOTONIC_CLOCK: &str = "wasi:clocks/monotonic-clock@0.2.12";

/// Replace the WASI monotonic clock in `linker` with one whose subscriptions
/// are ready at once, so a guest cannot sleep in a host call.
///
/// A guest blocked in a wait runs no wasm, so the epoch cannot stop it, and on
/// the sync backend the wait holds an OS thread the host can neither interrupt
/// nor reclaim. A controller runs in simulated time and has nothing to wait
/// for on the wall clock, so a wait returns immediately instead. `now` and
/// `resolution` read the clock as before.
///
/// The whole interface is defined again because a linker instance cannot be
/// reopened: naming it replaces it.
pub(super) fn clock_waits_return_at_once<T: wasmtime_wasi::WasiView + 'static>(
    linker: &mut Linker<T>,
) -> Result<(), PluginError> {
    use wasmtime_wasi::clocks::WasiClocksView;
    use wasmtime_wasi::p2::bindings::clocks::monotonic_clock::Host as _;

    fn ready<T: wasmtime_wasi::WasiView>(
        mut store: StoreContextMut<'_, T>,
    ) -> wasmtime::Result<(Resource<DynPollable>,)> {
        Ok((store.data_mut().clocks().subscribe_duration(0)?,))
    }

    linker.allow_shadowing(true);
    let defined = (|| -> wasmtime::Result<()> {
        let mut clock = linker.instance(MONOTONIC_CLOCK)?;
        clock.func_wrap("now", |mut store: StoreContextMut<'_, T>, (): ()| {
            Ok((store.data_mut().clocks().now()?,))
        })?;
        clock.func_wrap("resolution", |mut store: StoreContextMut<'_, T>, (): ()| {
            Ok((store.data_mut().clocks().resolution()?,))
        })?;
        clock.func_wrap("subscribe-instant", |store, (_when,): (u64,)| ready(store))?;
        clock.func_wrap("subscribe-duration", |store, (_duration,): (u64,)| {
            ready(store)
        })?;
        Ok(())
    })();
    linker.allow_shadowing(false);
    defined.map_err(|e| PluginError::Init(format!("WASI clock override failed: {e}")))
}

/// A wasmtime error as one line: the messages of its chain, without the wasm
/// backtrace wasmtime attaches to a trap.
///
/// The limits' own messages sit at the root of that chain, under the
/// backtrace context, so formatting only the outermost error would drop the
/// reason a guest was stopped.
pub(super) fn guest_error(e: &wasmtime::Error) -> String {
    let parts: Vec<String> = e
        .chain()
        .map(|cause| cause.to_string())
        .filter(|m| !m.starts_with("error while executing at wasm backtrace"))
        .collect();
    if parts.is_empty() {
        e.to_string()
    } else {
        parts.join(": ")
    }
}

/// [`ResourceLimiter`] enforcing a [`GuestLimits`]. Growth past a limit is an
/// error, which traps the guest with a message naming the limit.
#[derive(Debug)]
pub(super) struct GuestLimiter {
    limits: GuestLimits,
}

impl GuestLimiter {
    pub(super) fn new(limits: GuestLimits) -> Self {
        Self { limits }
    }
}

impl ResourceLimiter for GuestLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.limits.memory_bytes {
            return Err(wasmtime::format_err!(
                "the guest asked for a linear memory of {desired} bytes, above the \
                 {}-byte limit",
                self.limits.memory_bytes
            ));
        }
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.limits.table_elements {
            return Err(wasmtime::format_err!(
                "the guest asked for a table of {desired} elements, above the \
                 {}-element limit",
                self.limits.table_elements
            ));
        }
        Ok(true)
    }

    fn instances(&self) -> usize {
        self.limits.instances
    }

    fn tables(&self) -> usize {
        self.limits.tables
    }

    fn memories(&self) -> usize {
        self.limits.memories
    }
}

/// Bytes of host memory one msg-io outbound keeps: its kind, every string and
/// byte list in its payload, and the storage of each key-value field itself,
/// so a list of many small fields counts for what it holds.
///
/// A macro because the sync and async bindings generate separate `wit` types
/// of the same shape, and both host states count them the same way.
macro_rules! outbound_bytes {
    ($wit:ident, $msg:expr) => {{
        let msg = $msg;
        msg.kind.len()
            + match &msg.payload {
                $wit::Payload::Json(s) => s.len(),
                $wit::Payload::Binary(b) => b.len(),
                $wit::Payload::KeyValue(fields) => fields
                    .iter()
                    .map(|f| {
                        ::core::mem::size_of_val(f)
                            + f.name.len()
                            + match &f.value {
                                $wit::Value::Text(s) => s.len(),
                                $wit::Value::Bytes(b) => b.len(),
                                _ => 0,
                            }
                    })
                    .sum::<usize>(),
            }
    }};
}
pub(super) use outbound_bytes;

/// Counts the msg-io messages a guest sent in the current turn against
/// [`MAX_MESSAGES`] and [`MAX_MESSAGE_BYTES`].
#[derive(Debug, Default)]
pub(super) struct OutboxBudget {
    messages: usize,
    bytes: usize,
    /// Set once the guest passed a limit; later messages of the turn are
    /// dropped and the tick fails with this.
    fault: Option<String>,
}

impl OutboxBudget {
    /// Whether a message of `bytes` fits in this turn. Once one does not,
    /// none does, and [`Self::take_fault`] reports it.
    pub(super) fn admit(&mut self, bytes: usize) -> bool {
        if self.fault.is_some() {
            return false;
        }
        let messages = self.messages + 1;
        let total = self.bytes.saturating_add(bytes);
        if messages > MAX_MESSAGES || total > MAX_MESSAGE_BYTES {
            self.fault = Some(format!(
                "msg-io: the guest sent more than {MAX_MESSAGES} messages or \
                 {MAX_MESSAGE_BYTES} bytes of messages in one tick"
            ));
            return false;
        }
        self.messages = messages;
        self.bytes = total;
        true
    }

    /// End the turn: the fault, if the guest passed a limit, and a fresh count.
    pub(super) fn take_fault(&mut self) -> Option<String> {
        let fault = self.fault.take();
        self.messages = 0;
        self.bytes = 0;
        fault
    }
}

/// Counts the `host-env.log` records of the current turn against
/// [`MAX_LOG_RECORDS`].
#[derive(Debug, Default)]
pub(super) struct LogBudget {
    records: usize,
}

/// What to do with one guest log record.
pub(super) enum LogAdmit {
    /// Write it (cut to [`MAX_LOG_RECORD_BYTES`]).
    Write,
    /// Drop it, and write one warning that the rest of the turn's are dropped.
    DropAndNote,
    /// Drop it quietly: the warning went out already.
    Drop,
}

impl LogBudget {
    pub(super) fn admit(&mut self) -> LogAdmit {
        self.records += 1;
        match self.records.cmp(&(MAX_LOG_RECORDS + 1)) {
            core::cmp::Ordering::Less => LogAdmit::Write,
            core::cmp::Ordering::Equal => LogAdmit::DropAndNote,
            core::cmp::Ordering::Greater => LogAdmit::Drop,
        }
    }

    /// A new turn starts with a fresh count.
    pub(super) fn reset(&mut self) {
        self.records = 0;
    }
}

/// `message` in at most [`MAX_LOG_RECORD_BYTES`] bytes: cut at a character
/// boundary and marked with `…` (inside the limit) when longer.
pub(super) fn bounded_log_record(message: &str) -> std::borrow::Cow<'_, str> {
    const MARK: &str = "…";
    if message.len() <= MAX_LOG_RECORD_BYTES {
        return std::borrow::Cow::Borrowed(message);
    }
    let mut end = MAX_LOG_RECORD_BYTES - MARK.len();
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    std::borrow::Cow::Owned(format!("{}{MARK}", &message[..end]))
}

/// The msg-io messages a controller keeps for its caller between
/// `take_outbound` calls: a caller that never takes them halts the simulation
/// instead of growing without bound.
///
/// Twice a turn's limits, because the first tick's response carries two
/// turns: the messages sent at the start of `run`, before the first tick, and
/// those of the first tick. Every later response carries one, so a caller that
/// takes the messages after each tick never reaches the bound. `orts run` takes
/// them after every span, and a span holds at most one tick of each
/// satellite (it ends at the fleet's next tick); `orts serve` drops them after
/// every tick.
#[derive(Debug, Default)]
pub(super) struct OutboundBacklog {
    messages: Vec<crate::plugin::Message>,
    bytes: usize,
}

impl OutboundBacklog {
    /// Keep `msg`, or say that the caller has not been taking them.
    pub(super) fn push(&mut self, msg: crate::plugin::Message) -> Result<(), String> {
        let bytes = message_bytes(&msg);
        let (max_messages, max_bytes) = (2 * MAX_MESSAGES, 2 * MAX_MESSAGE_BYTES);
        if self.messages.len() + 1 > max_messages || self.bytes.saturating_add(bytes) > max_bytes {
            return Err(format!(
                "msg-io: outbound backlog overrun: more than {max_messages} messages or \
                 {max_bytes} bytes waiting (consumer not draining)"
            ));
        }
        self.bytes += bytes;
        self.messages.push(msg);
        Ok(())
    }

    /// Hand every kept message over and start empty.
    pub(super) fn take(&mut self) -> Vec<crate::plugin::Message> {
        self.bytes = 0;
        std::mem::take(&mut self.messages)
    }
}

fn message_bytes(msg: &crate::plugin::Message) -> usize {
    use crate::plugin::{Payload, Value};
    msg.kind.len()
        + match &msg.payload {
            Payload::Json(s) => s.len(),
            Payload::Binary(b) => b.len(),
            Payload::KeyValue(fields) => fields
                .iter()
                .map(|f| {
                    core::mem::size_of_val(f)
                        + f.name.len()
                        + match &f.value {
                            Value::Text(s) => s.len(),
                            Value::Bytes(b) => b.len(),
                            _ => 0,
                        }
                })
                .sum(),
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A turn writes [`MAX_LOG_RECORDS`] records, notes the drop once, and
    /// drops the rest quietly; the next turn writes again.
    #[test]
    fn a_turn_writes_at_most_the_log_limit() {
        let mut budget = LogBudget::default();
        let mut written = 0;
        let mut noted = 0;
        for _ in 0..MAX_LOG_RECORDS + 10 {
            match budget.admit() {
                LogAdmit::Write => written += 1,
                LogAdmit::DropAndNote => noted += 1,
                LogAdmit::Drop => {}
            }
        }
        assert_eq!((written, noted), (MAX_LOG_RECORDS, 1));
        budget.reset();
        assert!(matches!(budget.admit(), LogAdmit::Write));
    }

    /// A long record is cut at a character boundary and marked, the mark
    /// included in the limit; a short one passes as it is.
    #[test]
    fn a_long_log_record_is_cut_at_a_character_boundary() {
        assert_eq!(bounded_log_record("short"), "short");
        let exact = "a".repeat(MAX_LOG_RECORD_BYTES);
        assert_eq!(bounded_log_record(&exact), exact.as_str());

        let ascii = bounded_log_record(&"a".repeat(10_000)).into_owned();
        assert!(ascii.ends_with('…'), "marked as cut");
        assert_eq!(ascii.len(), MAX_LOG_RECORD_BYTES);

        // Three-byte characters: 4093 is not a boundary, 4092 is.
        let wide = bounded_log_record(&"あ".repeat(2000)).into_owned();
        assert!(wide.ends_with('…'));
        assert_eq!(wide.len(), 4092 + '…'.len_utf8());
        assert!(wide.len() <= MAX_LOG_RECORD_BYTES);
    }

    /// Each key-value field counts for its own storage, so a message of many
    /// empty fields is not free.
    #[test]
    fn empty_key_value_fields_still_count() {
        use crate::plugin::{Message, NamedValue, NodeId, Payload, Value};
        let fields = vec![
            NamedValue {
                name: String::new(),
                value: Value::Boolean(true),
            };
            1000
        ];
        let msg = Message {
            src: NodeId::Ground,
            dst: NodeId::Ground,
            kind: String::new(),
            payload: Payload::KeyValue(fields),
        };
        assert_eq!(
            message_bytes(&msg),
            1000 * core::mem::size_of::<NamedValue>()
        );
    }
}
