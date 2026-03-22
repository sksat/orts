import { createRoot } from "react-dom/client";
import { App } from "./App.js";

// biome-ignore lint/style/noNonNullAssertion: root element guaranteed by index.html
const root = createRoot(document.getElementById("root")!);
root.render(<App />);
