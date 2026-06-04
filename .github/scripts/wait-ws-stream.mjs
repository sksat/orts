// Wait until the orts WebSocket server is actually *streaming* simulation data,
// i.e. a connected client receives a `state` message — not merely that the TCP
// port is open.
//
// The previous CI gate (`nc -z localhost 9001`) passed as soon as the port was
// bound (`TcpListener::bind`), which happens before `axum::serve` starts and
// before the simulation manager is spawned/initialized. On connect the server
// only emits `info`/`state` after a `GetStatus` round-trip to that manager, so
// a client connecting in the gap receives nothing — the source of the flaky
// `viewer-e2e` WS tests. See https://github.com/sksat/orts/issues/65.
//
// Uses Node's global WebSocket (Node >= 22; repo pins 24.x), so no dependency.
//
// Usage: node .github/scripts/wait-ws-stream.mjs [ws-url]
//   env WS_WAIT_TIMEOUT_MS (default 60000) — overall budget before failing.

const url = process.argv[2] ?? "ws://localhost:9001/ws";
const parsedTimeout = Number(process.env.WS_WAIT_TIMEOUT_MS ?? 60000);
// Fall back to the default for a non-numeric / non-positive override, otherwise
// setTimeout(..., NaN) would fire immediately and the probe would falsely fail.
const overallTimeoutMs =
  Number.isFinite(parsedTimeout) && parsedTimeout > 0 ? parsedTimeout : 60000;
const retryDelayMs = 1000;
const deadline = Date.now() + overallTimeoutMs;

let attempt = 0;
let done = false;

function finish(code, msg) {
  if (done) return;
  done = true;
  if (code === 0) console.log(msg);
  else console.error(msg);
  process.exit(code);
}

setTimeout(
  () =>
    finish(1, `timed out after ${overallTimeoutMs}ms waiting for a 'state' message from ${url}`),
  overallTimeoutMs,
).unref();

function connectOnce() {
  if (done) return;
  attempt++;
  const ws = new WebSocket(url);

  ws.addEventListener("message", (e) => {
    let msg;
    try {
      msg = JSON.parse(e.data);
    } catch {
      return; // ignore non-JSON frames
    }
    // `state` confirms the simulation manager is initialized and broadcasting,
    // which is the readiness signal the E2E tests actually depend on.
    if (msg?.type === "state") {
      try {
        ws.close();
      } catch {}
      finish(0, `ready: received 'state' from ${url} (attempt ${attempt})`);
    }
  });

  // Connection refused / dropped before streaming: server not accepting yet —
  // retry until the overall deadline. A failed attempt fires `error` then
  // `close`, so guard with `retried` to schedule at most one retry per attempt;
  // otherwise overlapping connectOnce() calls would pile up extra sockets.
  let retried = false;
  const retry = () => {
    if (done || retried) return;
    retried = true;
    try {
      ws.close();
    } catch {}
    if (Date.now() >= deadline) {
      finish(1, `timed out waiting for 'state' from ${url}`);
      return;
    }
    setTimeout(connectOnce, retryDelayMs);
  };
  ws.addEventListener("error", retry);
  ws.addEventListener("close", retry);
}

// Global WebSocket exists on Node >= 22 (repo pins 24.x). Fail with a clear
// message rather than a bare ReferenceError if ever run on an older runtime.
if (typeof WebSocket === "undefined") {
  finish(1, "global WebSocket is unavailable; this probe requires Node >= 22");
}

connectOnce();
