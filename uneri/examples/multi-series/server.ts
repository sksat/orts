import { WebSocketServer } from "ws";

const PORT = 9004;

const wss = new WebSocketServer({ port: PORT });

console.log(`Multi-series WebSocket server listening on ws://localhost:${PORT}`);

wss.on("connection", (ws) => {
  console.log("Client connected");

  ws.send(
    JSON.stringify({
      type: "info",
      description: "Two sine waves with different frequencies",
    }),
  );

  const startTime = Date.now();
  let toggle = false;

  // Alternate between two series at different rates to produce
  // independent time arrays (as real multi-satellite data would).
  const interval = setInterval(() => {
    const t = (Date.now() - startTime) / 1000;
    toggle = !toggle;

    if (toggle) {
      ws.send(
        JSON.stringify({
          type: "state",
          series: "slow",
          t,
          value: Math.sin(t),
        }),
      );
    } else {
      ws.send(
        JSON.stringify({
          type: "state",
          series: "fast",
          t,
          value: Math.sin(t * 3),
        }),
      );
    }
  }, 50);

  ws.on("close", () => {
    console.log("Client disconnected");
    clearInterval(interval);
  });

  ws.on("error", (err) => {
    console.error("WebSocket error:", err);
    clearInterval(interval);
  });
});
