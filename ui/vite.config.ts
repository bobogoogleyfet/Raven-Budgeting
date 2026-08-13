import { defineConfig } from "vite";

// Tauri serves this dev server inside the webview and bundles `dist/` for
// release. The fixed port matters: `tauri.conf.json` names the same one, and a
// Vite fallback to :5174 when :5173 is busy would leave the window blank with
// no visible cause. `strictPort` turns that into an error instead.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    // Safari 13 is Tauri v2's documented floor for the WebKit-based webviews
    // (macOS and iOS); Linux and Android run newer engines, so this is the
    // binding constraint across the four targets.
    target: "safari13",
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: true,
  },
});
