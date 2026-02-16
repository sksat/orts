import { WebSocketServer } from "ws";

const PORT = 9005;

const wss = new WebSocketServer({ port: PORT });

console.log(`Multi-table WebSocket server listening on ws://localhost:${PORT}`);

wss.on("connection", (ws) => {
  console.log("Client connected");

  ws.send(
    JSON.stringify({
      type: "info",
      description: "Two series at same rate for multi-table DuckDB alignment test",
      tables: ["alpha", "beta"],
    }),
  );

  // Send both series at the same dt but as separate table entries.
  // Both start at t=0, advance by dt=0.5.
  // This tests that time-bucket downsampling produces aligned timestamps
  // when both tables share a unified tMax.
  let t = 0;
  const DT = 0.5;

  const interval = setInterval(() => {
    // alpha: sin wave
    ws.send(
      JSON.stringify({
        type: "state",
        table: "alpha",
        t,
        value: Math.sin(t * 0.1),
      }),
    );
    // beta: cos wave with offset
    ws.send(
      JSON.stringify({
        type: "state",
        table: "beta",
        t,
        value: Math.cos(t * 0.1) + 2,
      }),
    );
    t += DT;
  }, 10); // 100 points/sec per series

  ws.on("close", () => {
    console.log("Client disconnected");
    clearInterval(interval);
  });

  ws.on("error", (err) => {
    console.error("WebSocket error:", err);
    clearInterval(interval);
  });
});
