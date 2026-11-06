import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";
import { defineConfig } from "vite";

// Tauri expects a fixed dev port and serves the built `dist/` in release.
export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  resolve: {
    alias: { $lib: path.resolve("./src/lib") },
  },
  clearScreen: false,
  server: { host: "127.0.0.1", port: 5297, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  build: { target: "safari16", outDir: "dist", chunkSizeWarningLimit: 2000 },
});
