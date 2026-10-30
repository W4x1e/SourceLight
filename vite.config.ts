/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri expects a fixed port and does not want vite to fall back to another one.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // src-tauri is watched by cargo, not vite.
      ignored: ["**/src-tauri/**"],
    },
  },
  test: {
    // jsdom for the component/hook tests; the pure-logic tests need no DOM but
    // pay nothing for it. Real browser verification stays the preview pane's
    // job — this is for logic that used to need a thrown-away harness.
    environment: "jsdom",
    globals: true,
    // The phone app's own logic too (mobile/src), which shares this code.
    include: ["src/**/*.{test,spec}.{ts,tsx}", "mobile/src/**/*.{test,spec}.{ts,tsx}"],
    // src-tauri holds the Rust suite; vitest must not wander into it.
    exclude: ["src-tauri/**", "node_modules/**", "dist/**"],
    // A test that loads a locale took 1.6 s alone and passed the 5 s default
    // while cargo was compiling alongside.
    testTimeout: 15000,
  },
});
