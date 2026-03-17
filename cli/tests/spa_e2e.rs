//! E2E tests for embedded SPA serving.
//!
//! These tests verify that `orts serve` correctly serves the embedded viewer
//! assets via the SPA fallback handler, including index.html, hashed assets,
//! Cache-Control headers, and SPA client-side routing fallback.
//!
//! All tests are gated on `#[cfg(feature = "viewer")]` — when built with
//! `--no-default-features` these tests are compiled out entirely.

#![cfg(feature = "viewer")]

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Pick a port unlikely to collide with other test processes.
/// Offset from ws_e2e's 19000 range to avoid conflicts.
fn test_port() -> u16 {
    let pid = std::process::id();
    20000 + (pid % 1000) as u16
}

struct Server {
    child: std::process::Child,
    _stderr_thread: std::thread::JoinHandle<()>,
}

impl Server {
    fn spawn_idle(port: u16) -> Self {
        let binary = env!("CARGO_BIN_EXE_orts");
        let args = vec!["serve".to_string(), "--port".to_string(), port.to_string()];
        let mut child = Command::new(binary)
            .args(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn orts");

        let stderr = child.stderr.take().expect("failed to capture stderr");
        let (tx, rx) = mpsc::channel::<()>();

        let stderr_thread = std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            let mut notified = false;
            for line in reader.lines() {
                let line = line.expect("failed to read stderr line");
                eprintln!("[server stderr] {line}");
                if !notified && line.contains("Server listening on") {
                    let _ = tx.send(());
                    notified = true;
                }
            }
            if !notified {
                let _ = tx.send(());
            }
        });

        rx.recv_timeout(Duration::from_secs(10))
            .expect("server did not print 'listening' message within 10 seconds");

        Server {
            child,
            _stderr_thread: stderr_thread,
        }
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Simple HTTP response wrapper for ureq v3.
struct HttpResponse {
    status: u16,
    body: String,
    headers: Vec<(String, String)>,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        let name_lower = name.to_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.to_lowercase() == name_lower)
            .map(|(_, v)| v.as_str())
    }
}

fn http_get(port: u16, path: &str) -> HttpResponse {
    let url = format!("http://localhost:{port}{path}");
    match ureq::get(&url).call() {
        Ok(mut resp) => {
            let status: u16 = resp.status().into();
            let headers: Vec<(String, String)> = resp
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();
            let body = resp.body_mut().read_to_string().unwrap_or_default();
            HttpResponse {
                status,
                body,
                headers,
            }
        }
        Err(ureq::Error::StatusCode(code)) => HttpResponse {
            status: code,
            body: String::new(),
            headers: Vec::new(),
        },
        Err(e) => panic!("GET {path} failed: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Tests with viewer feature (default build)
// ---------------------------------------------------------------------------

#[test]
fn spa_root_serves_index_html() {
    let port = test_port();
    let mut server = Server::spawn_idle(port);

    let resp = http_get(port, "/");
    assert_eq!(resp.status, 200);

    let ct = resp.header("content-type").unwrap_or("");
    assert!(ct.contains("text/html"), "expected text/html, got {ct}");

    let cache = resp.header("cache-control").unwrap_or("");
    assert_eq!(cache, "no-cache", "index.html should have no-cache");

    assert!(
        resp.body.contains("<div id=\"root\">") || resp.body.contains("Viewer not built"),
        "index.html should contain viewer root or placeholder"
    );

    server.kill();
}

#[test]
fn spa_unknown_path_falls_back_to_index_html() {
    let port = test_port() + 1;
    let mut server = Server::spawn_idle(port);

    let resp = http_get(port, "/some/random/path");
    assert_eq!(resp.status, 200);

    let ct = resp.header("content-type").unwrap_or("");
    assert!(ct.contains("text/html"), "SPA fallback should serve HTML");

    assert!(
        resp.body.contains("<div id=\"root\">") || resp.body.contains("Viewer not built"),
        "SPA fallback should serve index.html content"
    );

    server.kill();
}

#[test]
fn spa_textures_not_served_by_spa_handler() {
    let port = test_port() + 2;
    let mut server = Server::spawn_idle(port);

    // Embedded 2K textures should be served by the texture handler, not SPA
    let resp = http_get(port, "/textures/earth_2k.jpg");
    assert_eq!(resp.status, 200);

    let ct = resp.header("content-type").unwrap_or("");
    assert!(
        ct.contains("image/jpeg"),
        "texture should be image/jpeg, got {ct}"
    );

    // Non-existent texture returns 404 (not SPA fallback)
    let resp = http_get(port, "/textures/nonexistent.jpg");
    assert_eq!(resp.status, 404, "missing texture should return 404");

    server.kill();
}

#[test]
fn spa_ws_endpoint_not_hijacked() {
    let port = test_port() + 3;
    let mut server = Server::spawn_idle(port);

    // /ws with plain HTTP (no upgrade) should NOT return SPA fallback (200 + HTML)
    let resp = http_get(port, "/ws");
    // axum's WS handler returns non-200 for plain HTTP requests
    assert_ne!(resp.status, 200, "/ws plain HTTP should not return 200");

    server.kill();
}

#[test]
fn spa_cache_headers_for_assets() {
    let port = test_port() + 4;
    let mut server = Server::spawn_idle(port);

    // Get index.html and extract an actual asset path from it
    let resp = http_get(port, "/");
    let body = &resp.body;

    // Try to extract an actual asset path from index.html
    if let Some(start) = body.find("/assets/") {
        let rest = &body[start..];
        if let Some(end) = rest.find('"').or_else(|| rest.find('\'')) {
            let asset_path = &rest[..end];
            let resp = http_get(port, asset_path);
            assert_eq!(resp.status, 200, "asset {asset_path} should exist");

            let cache = resp.header("cache-control").unwrap_or("");
            assert!(
                cache.contains("immutable"),
                "hashed assets should have immutable cache: got {cache}"
            );
        }
    }

    server.kill();
}
