import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Standalone consumer app for the orbit-viewer registry item. `@` resolves to
// this package root so `@/components/orbit-viewer/...` (where `shadcn add` writes
// the copied source) imports the registry-distributed component tree.
export default defineConfig({
  plugins: [react()],
  // Serve the viewer's static textures (/textures/earth_2k.jpg, …) that the
  // copied default bodies request, so the example renders the textured output
  // rather than flat fallback colours.
  publicDir: fileURLToPath(new URL("../../public", import.meta.url)),
  resolve: {
    alias: {
      "@": fileURLToPath(new URL(".", import.meta.url)),
    },
  },
});
