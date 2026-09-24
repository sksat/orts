//! Controller components a WebSocket client sends to `orts serve`.
//!
//! A client does not name a controller by a path on the server: the server
//! would open whatever file the client asked for, and a FIFO there held the
//! manager in a read that never returned (#556). The client sends the
//! component's bytes instead, as one binary message on `/ws`, and a controller
//! config names it by the SHA-256 the server replies with:
//!
//! ```json
//! {"type": "wasm", "sha256": "<64 lowercase hex digits>", "config": {...}}
//! ```
//!
//! A connection keeps the components it received until it closes, and a
//! message can name only those. A config file given to `orts serve` on its
//! command line still names a controller by `path`: the person starting the
//! server wrote it.
//!
//! Uploading runs client code on the server, so it is off unless the server
//! was started with `--allow-controller-upload`. Without the flag a binary
//! message and a `sha256` controller are both refused, before the bytes are
//! hashed or kept.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::config::{ControllerConfig, SatelliteConfig, SimConfig};

/// The largest controller component a client may send.
///
/// Release builds of the example guests in `plugin-sdk/examples` are 84 KB to
/// 199 KB, and the largest `.wasm` measured, a debug build of the C-based
/// `nos3-adcs` example, is 4.3 MB. 8 MiB leaves room for that one.
pub(super) const MAX_CONTROLLER_COMPONENT_BYTES: usize = 8 * 1024 * 1024;

/// How many distinct components one connection keeps.
///
/// With [`MAX_CONTROLLER_COMPONENT_BYTES`] this bounds what one connection
/// holds to 32 MiB. A fleet usually shares one controller, and a mission a
/// handful. Once full, a new component is refused rather than one evicted: an
/// eviction would drop a component the client was told it could name.
#[cfg(feature = "plugin-wasm")]
pub(super) const MAX_COMPONENTS_PER_CONNECTION: usize = 4;

/// The most component bytes all connections hold together.
///
/// The per-connection limit alone does not bound the server: `/ws` takes any
/// number of unauthenticated connections, and each could hold its 32 MiB for as
/// long as it stays open. 128 MiB is four connections at their limit, or
/// hundreds of the example guests' release builds. A connection gives its share
/// back when it closes. A component a running simulation built a satellite
/// from stays with that satellite and is not counted here: keeping it takes
/// adding a satellite, which is how any other part of a fleet grows too.
pub(super) const MAX_UPLOADED_BYTES_PER_SERVER: usize = 128 * 1024 * 1024;

/// The first 8 bytes of a WASM component: `\0asm`, the component-model
/// version `0x0d`, and layer 1. A core module has layer 0 and version 1.
///
/// <https://github.com/WebAssembly/component-model/blob/main/design/mvp/Binary.md>
const COMPONENT_PREAMBLE: [u8; 8] = [0x00, 0x61, 0x73, 0x6d, 0x0d, 0x00, 0x01, 0x00];

/// Why a controller given over WebSocket names a path, and what to do instead.
const PATH_REFUSAL: &str = "`path` is not accepted over WebSocket: send the \
     component's bytes as a binary message on /ws to a server started with \
     --allow-controller-upload, and name it by the `sha256` the server replies with; \
     a controller named by path comes with the `--config` file `orts serve` is started \
     with";

/// Why a server started without the flag refuses an upload or a `sha256`.
pub(super) const UPLOAD_NOT_ALLOWED: &str = "WebSocket WASM controllers require starting \
     orts serve with --allow-controller-upload";

/// Refuse every controller in `config` that names a path.
///
/// The path is not repeated back: it is the client's own text, of any length.
pub(super) fn refuse_controller_paths(config: &SimConfig) -> Result<(), String> {
    for (i, sat) in config.satellites.iter().enumerate() {
        refuse_controller_path(sat, &format!("satellites[{i}]"))?;
    }
    Ok(())
}

/// Refuse `sat`'s controller if it names a path. `whose` says which satellite
/// in the error.
pub(super) fn refuse_controller_path(sat: &SatelliteConfig, whose: &str) -> Result<(), String> {
    match &sat.controller {
        Some(ControllerConfig::Wasm { path: Some(_), .. }) => {
            Err(format!("{whose}.controller: {PATH_REFUSAL}"))
        }
        _ => Ok(()),
    }
}

/// What the server tells a client whose upload it kept.
#[derive(Debug, PartialEq)]
pub(super) struct Uploaded {
    pub sha256: String,
    pub size: usize,
}

/// Component bytes the server's connections hold, against a limit shared by
/// all of them.
pub(super) struct UploadBudget {
    // Read only where uploads are kept, which takes `plugin-wasm`.
    #[cfg_attr(not(feature = "plugin-wasm"), allow(dead_code))]
    limit: usize,
    held: AtomicUsize,
}

impl UploadBudget {
    pub(super) fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit,
            held: AtomicUsize::new(0),
        })
    }

    /// Count `n` more bytes, unless that would pass the limit.
    #[cfg(feature = "plugin-wasm")]
    fn try_reserve(&self, n: usize) -> bool {
        self.held
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                held.checked_add(n).filter(|total| *total <= self.limit)
            })
            .is_ok()
    }

    fn release(&self, n: usize) {
        self.held.fetch_sub(n, Ordering::AcqRel);
    }
}

/// The components one `/ws` connection has received.
pub(super) struct UploadedComponents {
    #[cfg(feature = "plugin-wasm")]
    by_sha256: std::collections::HashMap<String, orts::plugin::wasm::ComponentBytes>,
    /// `--allow-controller-upload`.
    allowed: bool,
    budget: Arc<UploadBudget>,
    /// Bytes this connection counts against `budget`, given back on drop.
    reserved: usize,
}

impl Drop for UploadedComponents {
    fn drop(&mut self) {
        self.budget.release(self.reserved);
    }
}

impl UploadedComponents {
    /// An empty list whose uploads count against `budget`, and are refused
    /// unless `allowed`.
    pub(super) fn new(budget: Arc<UploadBudget>, allowed: bool) -> Self {
        Self {
            #[cfg(feature = "plugin-wasm")]
            by_sha256: std::collections::HashMap::new(),
            allowed,
            budget,
            reserved: 0,
        }
    }

    /// Keep the component in `bytes`, or say why not.
    ///
    /// Accepting checks the size, the component preamble, and the per-connection
    /// and server-wide limits, and nothing more: whether the bytes compile, link
    /// against the plugin interface, and run is found out when a controller is
    /// first built from them. Sending the same bytes again is accepted even when
    /// full, and counted once.
    pub(super) fn upload(&mut self, bytes: &[u8]) -> Result<Uploaded, String> {
        if !self.allowed {
            return Err(UPLOAD_NOT_ALLOWED.to_string());
        }
        if bytes.len() > MAX_CONTROLLER_COMPONENT_BYTES {
            return Err(format!(
                "a controller component is at most {MAX_CONTROLLER_COMPONENT_BYTES} bytes; \
                 this one is {}",
                bytes.len()
            ));
        }
        if !bytes.starts_with(&COMPONENT_PREAMBLE) {
            return Err(
                "a binary message on /ws is a controller component, and this one does not \
                 start as a WASM component does (`\\0asm`, version 0x0d, layer 1); a core \
                 module has to be made a component first, as `cargo component build` does"
                    .to_string(),
            );
        }
        self.keep(bytes)
    }

    #[cfg(feature = "plugin-wasm")]
    fn keep(&mut self, bytes: &[u8]) -> Result<Uploaded, String> {
        let component = orts::plugin::wasm::ComponentBytes::new(bytes);
        let sha256 = component.sha256_hex();
        let size = component.len();
        if !self.by_sha256.contains_key(&sha256) {
            if self.by_sha256.len() >= MAX_COMPONENTS_PER_CONNECTION {
                return Err(format!(
                    "this connection already holds {MAX_COMPONENTS_PER_CONNECTION} controller \
                     components, the most one connection keeps; a new connection starts \
                     with none"
                ));
            }
            if !self.budget.try_reserve(size) {
                return Err(format!(
                    "the server's connections already hold close to {} bytes of controller \
                     components, the most it keeps at once; a connection gives its \
                     components back when it closes",
                    self.budget.limit
                ));
            }
            self.reserved += size;
            self.by_sha256.insert(sha256.clone(), component);
        }
        Ok(Uploaded { sha256, size })
    }

    #[cfg(not(feature = "plugin-wasm"))]
    fn keep(&mut self, _bytes: &[u8]) -> Result<Uploaded, String> {
        Err("this orts was built without WASM controllers (the `plugin-wasm` feature)".to_string())
    }

    /// Check every controller in `config` and attach the bytes its `sha256`
    /// names.
    pub(super) fn resolve_config(&self, config: &mut SimConfig) -> Result<(), String> {
        for (i, sat) in config.satellites.iter_mut().enumerate() {
            self.resolve_satellite(sat, &format!("satellites[{i}]"))?;
        }
        Ok(())
    }

    /// Check `sat`'s controller and attach the bytes its `sha256` names.
    ///
    /// A path is refused before anything else is looked at, so no path a
    /// client sends reaches the filesystem, whatever else is wrong with the
    /// message.
    pub(super) fn resolve_satellite(
        &self,
        sat: &mut SatelliteConfig,
        whose: &str,
    ) -> Result<(), String> {
        refuse_controller_path(sat, whose)?;
        let Some(ControllerConfig::Wasm {
            sha256: Some(sha256),
            ..
        }) = &sat.controller
        else {
            return Ok(());
        };
        if !self.allowed {
            return Err(format!("{whose}.controller: {UPLOAD_NOT_ALLOWED}"));
        }
        if !crate::config::is_sha256_hex(sha256) {
            return Err(format!(
                "{whose}.controller: `sha256` must be 64 lowercase hexadecimal digits"
            ));
        }
        self.attach(sat, whose)
    }

    #[cfg(feature = "plugin-wasm")]
    fn attach(&self, sat: &mut SatelliteConfig, whose: &str) -> Result<(), String> {
        let Some(ControllerConfig::Wasm {
            sha256: Some(sha256),
            uploaded,
            ..
        }) = &mut sat.controller
        else {
            return Ok(());
        };
        match self.by_sha256.get(sha256.as_str()) {
            Some(component) => {
                *uploaded = Some(component.clone());
                Ok(())
            }
            None => Err(format!(
                "{whose}.controller: sha256 {sha256} names no component sent on this \
                 connection; send the component's bytes as a binary message on /ws first"
            )),
        }
    }

    #[cfg(not(feature = "plugin-wasm"))]
    fn attach(&self, _sat: &mut SatelliteConfig, whose: &str) -> Result<(), String> {
        Err(format!(
            "{whose}.controller: this orts was built without WASM controllers \
             (the `plugin-wasm` feature)"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A connection's list on a server started with `--allow-controller-upload`,
    /// with the server's whole budget to itself.
    fn store() -> UploadedComponents {
        UploadedComponents::new(UploadBudget::new(MAX_UPLOADED_BYTES_PER_SERVER), true)
    }

    /// Without `--allow-controller-upload` an upload and a `sha256` are both
    /// refused, naming the flag; a `path` is still refused as a path.
    #[test]
    fn without_the_flag_uploads_and_digests_are_refused() {
        let mut store =
            UploadedComponents::new(UploadBudget::new(MAX_UPLOADED_BYTES_PER_SERVER), false);
        let mut bytes = COMPONENT_PREAMBLE.to_vec();
        bytes.extend_from_slice(&[0; 16]);
        let err = store
            .upload(&bytes)
            .expect_err("no upload without the flag");
        assert_eq!(err, UPLOAD_NOT_ALLOWED);

        let mut sat = controlled_satellite(serde_json::json!({
            "type": "wasm", "sha256": "0".repeat(64)
        }));
        let err = store
            .resolve_satellite(&mut sat, "add_satellite")
            .expect_err("no digest without the flag");
        assert!(err.contains("--allow-controller-upload"), "{err}");

        let mut sat = controlled_satellite(serde_json::json!({
            "type": "wasm", "path": "ctrl.wasm"
        }));
        let err = store
            .resolve_satellite(&mut sat, "add_satellite")
            .expect_err("a path is refused either way");
        assert!(err.contains("not accepted over WebSocket"), "{err}");
    }

    fn controlled_satellite(controller: serde_json::Value) -> SatelliteConfig {
        serde_json::from_value(serde_json::json!({
            "id": "a",
            "orbit": { "type": "circular", "altitude": 500.0 },
            "attitude": { "inertia_diag": [10.0, 10.0, 10.0], "mass": 50.0 },
            "controller": controller,
        }))
        .expect("a valid satellite")
    }

    /// A controller naming a path is refused, and the error does not repeat
    /// the path.
    #[test]
    fn a_controller_path_is_refused() {
        let store = store();
        let mut sat = controlled_satellite(serde_json::json!({
            "type": "wasm", "path": "/tmp/ctrl.fifo"
        }));
        let err = store
            .resolve_satellite(&mut sat, "add_satellite")
            .expect_err("a path from a client is refused");
        assert!(err.contains("not accepted over WebSocket"), "{err}");
        assert!(err.starts_with("add_satellite.controller"), "{err}");
        assert!(!err.contains("ctrl.fifo"), "the path is not echoed: {err}");
    }

    /// A path next to a `sha256` is still refused: the path check comes first.
    #[test]
    fn a_path_beside_a_sha256_is_refused() {
        let store = store();
        let mut sat = controlled_satellite(serde_json::json!({
            "type": "wasm", "path": "ctrl.wasm", "sha256": "0".repeat(64)
        }));
        let err = store
            .resolve_satellite(&mut sat, "add_satellite")
            .expect_err("the path is refused whatever else is there");
        assert!(err.contains("not accepted over WebSocket"), "{err}");
    }

    /// Every satellite of a `start_simulation` is checked, not only the first.
    #[test]
    fn every_satellite_of_a_start_is_checked() {
        let config: SimConfig = serde_json::from_value(serde_json::json!({
            "satellites": [
                { "id": "a", "orbit": { "type": "circular", "altitude": 500.0 } },
                { "id": "b", "orbit": { "type": "circular", "altitude": 600.0 },
                  "attitude": { "inertia_diag": [10.0, 10.0, 10.0], "mass": 50.0 },
                  "controller": { "type": "wasm", "path": "ctrl.wasm" } }
            ]
        }))
        .expect("a valid config");
        let err = refuse_controller_paths(&config).expect_err("the second one names a path");
        assert!(err.starts_with("satellites[1].controller"), "{err}");

        let mut config = config;
        let err = store()
            .resolve_config(&mut config)
            .expect_err("and so does resolving it");
        assert!(err.starts_with("satellites[1].controller"), "{err}");
    }

    /// Bytes that do not start as a component does are refused; a core module
    /// is named as the likely mistake.
    #[test]
    fn bytes_that_are_not_a_component_are_refused() {
        let mut store = store();
        let core_module = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x00];
        for bytes in [&core_module[..], b"hello", &COMPONENT_PREAMBLE[..7], &[]] {
            let err = store.upload(bytes).expect_err("not a component");
            assert!(err.contains("core module"), "{err}");
        }
    }

    /// Bytes past the size limit are refused before anything else.
    #[test]
    fn a_component_past_the_size_limit_is_refused() {
        let mut store = store();
        let mut bytes = COMPONENT_PREAMBLE.to_vec();
        bytes.resize(MAX_CONTROLLER_COMPONENT_BYTES + 1, 0);
        let err = store.upload(&bytes).expect_err("one byte over");
        assert!(
            err.contains(&MAX_CONTROLLER_COMPONENT_BYTES.to_string()),
            "{err}"
        );
    }

    #[cfg(feature = "plugin-wasm")]
    mod with_plugin_wasm {
        use super::*;

        /// A stand-in for a component: the preamble, then bytes nobody compiles.
        fn fake_component(tag: u8) -> Vec<u8> {
            let mut bytes = COMPONENT_PREAMBLE.to_vec();
            bytes.extend_from_slice(&[tag; 16]);
            bytes
        }

        /// An upload is answered with the SHA-256 of its bytes, and a
        /// controller naming that digest gets those bytes attached.
        #[test]
        fn an_uploaded_component_is_attached_by_its_digest() {
            let mut store = store();
            let bytes = fake_component(7);
            let up = store.upload(&bytes).expect("a component is kept");
            assert_eq!(
                up.sha256,
                orts::plugin::wasm::ComponentBytes::new(bytes.clone()).sha256_hex()
            );
            assert_eq!(up.size, bytes.len());

            let mut sat = controlled_satellite(serde_json::json!({
                "type": "wasm", "sha256": up.sha256
            }));
            store
                .resolve_satellite(&mut sat, "add_satellite")
                .expect("the digest names a component on this connection");
            let Some(ControllerConfig::Wasm {
                uploaded: Some(component),
                ..
            }) = &sat.controller
            else {
                panic!("bytes attached: {:?}", sat.controller);
            };
            assert_eq!(component.bytes(), &bytes[..]);
        }

        /// A digest no upload on this connection produced is refused; another
        /// connection's store does not answer for it.
        #[test]
        fn a_digest_from_another_connection_is_refused() {
            let mut elsewhere = store();
            let up = elsewhere.upload(&fake_component(3)).expect("kept there");

            let here = store();
            let mut sat = controlled_satellite(serde_json::json!({
                "type": "wasm", "sha256": up.sha256
            }));
            let err = here
                .resolve_satellite(&mut sat, "add_satellite")
                .expect_err("nothing was sent on this connection");
            assert!(
                err.contains("names no component sent on this connection"),
                "{err}"
            );
        }

        /// A `sha256` that is not 64 lowercase hex digits is refused without
        /// being repeated back.
        #[test]
        fn a_malformed_digest_is_refused() {
            let store = store();
            let long = "z".repeat(10_000);
            for bad in ["ABCDEF", long.as_str(), &"A".repeat(64)] {
                let mut sat = controlled_satellite(serde_json::json!({
                    "type": "wasm", "sha256": bad
                }));
                let err = store
                    .resolve_satellite(&mut sat, "add_satellite")
                    .expect_err("not a SHA-256");
                assert!(err.contains("64 lowercase hexadecimal digits"), "{err}");
                assert!(err.len() < 200, "bounded: {} bytes", err.len());
            }
        }

        /// A fifth distinct component is refused and the four already kept
        /// stay; sending one of those again is still accepted.
        #[test]
        fn a_full_connection_refuses_a_new_component_and_keeps_the_rest() {
            let mut store = store();
            let kept: Vec<Uploaded> = (0..MAX_COMPONENTS_PER_CONNECTION as u8)
                .map(|i| store.upload(&fake_component(i)).expect("room left"))
                .collect();
            let err = store
                .upload(&fake_component(200))
                .expect_err("no room for a fifth");
            assert!(
                err.contains(&MAX_COMPONENTS_PER_CONNECTION.to_string()),
                "{err}"
            );
            assert_eq!(
                store.upload(&fake_component(0)).expect("already kept"),
                kept[0],
                "the same bytes again are the same component"
            );
            for up in &kept {
                let mut sat = controlled_satellite(serde_json::json!({
                    "type": "wasm", "sha256": up.sha256
                }));
                store
                    .resolve_satellite(&mut sat, "add_satellite")
                    .expect("every kept component still resolves");
            }
        }

        /// Connections share one budget: once it is spent, a new component is
        /// refused on every connection, and a connection that closes gives its
        /// share back. Sending a kept component again costs nothing.
        #[test]
        fn connections_share_one_budget_and_give_it_back_on_close() {
            let one = fake_component(1);
            let budget = UploadBudget::new(2 * one.len());
            let mut first = UploadedComponents::new(Arc::clone(&budget), true);
            first.upload(&fake_component(1)).expect("room for one");
            first
                .upload(&fake_component(1))
                .expect("the same bytes cost nothing");

            let mut second = UploadedComponents::new(Arc::clone(&budget), true);
            second
                .upload(&fake_component(2))
                .expect("room for a second");
            let err = second
                .upload(&fake_component(3))
                .expect_err("the budget is spent");
            assert!(err.contains("the most it keeps at once"), "{err}");

            drop(first);
            second
                .upload(&fake_component(3))
                .expect("the closed connection gave its share back");
        }

        /// The attached bytes are not something a client can send: a
        /// controller carrying an `uploaded` key is refused as unknown.
        #[test]
        fn the_attached_bytes_cannot_come_from_the_wire() {
            let err = serde_json::from_value::<ControllerConfig>(serde_json::json!({
                "type": "wasm", "sha256": "0".repeat(64), "uploaded": [0, 97, 115, 109]
            }))
            .expect_err("`uploaded` is not a wire field");
            assert!(
                err.to_string().contains("unknown field `uploaded`"),
                "{err}"
            );
        }
    }
}
