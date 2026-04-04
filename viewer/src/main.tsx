import { createRoot } from "react-dom/client";
import "./styles/tokens.css";
import "./styles/base.css";
import "uneri/style.css";
import { App } from "./App.js";

// React 19 dev mode accumulates performance.measure entries via logComponentRender,
// which causes OOM on long-running sessions. Clear them periodically.
if (import.meta.env.DEV) {
  const id = setInterval(() => performance.clearMeasures(), 10_000);
  if (import.meta.hot) {
    import.meta.hot.dispose(() => clearInterval(id));
  }
}

const root = document.getElementById("root");
if (!root) {
  throw new Error("Root element not found");
}

createRoot(root).render(<App />);
