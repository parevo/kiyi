import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// https://v2.tauri.app/start/frontend/vite/
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**"] },
    // Browser-only development: src/dev/bridge.ts talks to kiyi-devbridge through this.
    proxy: { "/__bridge": { target: "http://127.0.0.1:1421", rewrite: (path) => path.replace(/^\/__bridge/, "") } },
  },
  build: {
    target: "safari15",
    sourcemap: false,
    // Loaded from disk by the webview, so one larger chunk costs nothing.
    chunkSizeWarningLimit: 2000,
  },
});
