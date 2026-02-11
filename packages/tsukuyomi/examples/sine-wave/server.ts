import { WebSocketServer } from "ws";

const PORT = 9002;

const wss = new WebSocketServer({ port: PORT });

console.log(`Sine wave WebSocket server listening on ws://localhost:${PORT}`);

wss.on("connection", (ws) => {
  console.log("Client connected");

  // Send info message
  ws.send(
    JSON.stringify({
      type: "info",
      description: "Sine wave generator",
      dt: 0.1,
      stream_interval: 0.1,
    }),
  );

  const startTime = Date.now();

  const interval = setInterval(() => {
    const t = (Date.now() - startTime) / 1000;
    ws.send(
      JSON.stringify({
        type: "state",
        t,
        value: Math.sin(t),
        derivative: Math.cos(t),
      }),
    );
  }, 100);

  ws.on("close", () => {
    console.log("Client disconnected");
    clearInterval(interval);
  });

  ws.on("error", (err) => {
    console.error("WebSocket error:", err);
    clearInterval(interval);
  });
});
