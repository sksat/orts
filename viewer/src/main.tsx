import { createRoot } from "react-dom/client";
import "uneri/style.css";
import { App } from "./App.js";

const root = document.getElementById("root");
if (!root) {
  throw new Error("Root element not found");
}

createRoot(root).render(<App />);
