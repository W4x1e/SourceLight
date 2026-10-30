import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// The phone app's interface (docs/MOBILE.md). It shares the desktop's styles,
// translations and pure logic by importing from ../src. Port 1430, so it never
// collides with the desktop's 1420.
//
// On a real phone the dev server must be reachable over the network: the
// Tauri CLI sets TAURI_DEV_HOST to this computer's address for that.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  root: fileURLToPath(new URL(".", import.meta.url)),
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1430,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1431 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
});
