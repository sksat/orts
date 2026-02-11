import { WebSocketServer } from "ws";

const PORT = 9003;

const wss = new WebSocketServer({ port: PORT });

console.log(
  `Mixed-density WebSocket server listening on ws://localhost:${PORT}`,
);

wss.on("connection", (ws) => {
  console.log("Client connected");

  // Send info message
  ws.send(
    JSON.stringify({
      type: "info",
      description: "Mixed-density generator (sparse overview + dense stream)",
      dt: 0.1,
      stream_interval: 0.1,
    }),
  );

  // Phase 1: Send 100 sparse "overview" points immediately (t=0..4950, step=50)
  // This simulates the server's history overview message.
  for (let i = 0; i < 100; i++) {
    const t = i * 50;
    ws.send(
      JSON.stringify({
        type: "state",
        t,
        value: Math.sin(t * 0.001),
        derivative: Math.cos(t * 0.001),
      }),
    );
  }
  console.log("Sent 100 sparse overview points (t=0..4950)");

  // Phase 2: Dense streaming starting at t=5000, advancing by 0.1 per message
  let streamT = 5000;
  const interval = setInterval(() => {
    ws.send(
      JSON.stringify({
        type: "state",
        t: streamT,
        value: Math.sin(streamT * 0.001),
        derivative: Math.cos(streamT * 0.001),
      }),
    );
    streamT += 0.1;
  }, 10); // 100 messages/sec

  ws.on("close", () => {
    console.log("Client disconnected");
    clearInterval(interval);
  });

  ws.on("error", (err) => {
    console.error("WebSocket error:", err);
    clearInterval(interval);
  });
});
