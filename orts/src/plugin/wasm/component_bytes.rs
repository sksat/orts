//! A WASM component held as bytes rather than read from a file.

use std::fmt;
use std::sync::Arc;

use sha2::{Digest, Sha256};

/// The bytes of a WASM component, together with their SHA-256.
///
/// [`WasmPluginCache`](super::WasmPluginCache) compiles one component per
/// digest. The digest is computed here, from the bytes it names, so a caller
/// cannot hand the cache one digest with another component's bytes and have
/// the first compilation answer for both.
///
/// Cloning shares the bytes.
#[derive(Clone)]
pub struct ComponentBytes {
    bytes: Arc<[u8]>,
    sha256: [u8; 32],
}

impl ComponentBytes {
    /// Take `bytes` and compute their SHA-256.
    ///
    /// Nothing here checks that the bytes are a component: compiling them is
    /// what does, when a controller is first built from them.
    pub fn new(bytes: impl Into<Arc<[u8]>>) -> Self {
        let bytes = bytes.into();
        let sha256 = Sha256::digest(&bytes).into();
        Self { bytes, sha256 }
    }

    /// The component's bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// SHA-256 of [`bytes`](Self::bytes).
    pub fn sha256(&self) -> &[u8; 32] {
        &self.sha256
    }

    /// SHA-256 of [`bytes`](Self::bytes) as 64 lowercase hexadecimal digits.
    pub fn sha256_hex(&self) -> String {
        self.sha256.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Length of the component in bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether there are no bytes at all.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// The digest and the length, not the bytes: a component is up to megabytes.
impl fmt::Debug for ComponentBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ComponentBytes")
            .field("sha256", &self.sha256_hex())
            .field("len", &self.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The digest is SHA-256 of the bytes, spelled in lowercase hex.
    ///
    /// The expected value is the FIPS 180-2 test vector for "abc".
    #[test]
    fn the_digest_is_the_sha256_of_the_bytes() {
        let c = ComponentBytes::new(b"abc".to_vec());
        assert_eq!(
            c.sha256_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(c.len(), 3);
    }

    /// `Debug` names the component by digest and length; the bytes stay out.
    #[test]
    fn debug_leaves_the_bytes_out() {
        let c = ComponentBytes::new(vec![0x5a_u8; 4096]);
        let shown = format!("{c:?}");
        assert!(shown.contains(&c.sha256_hex()), "{shown}");
        assert!(shown.contains("4096"), "{shown}");
        assert!(shown.len() < 200, "{shown}");
    }
}
