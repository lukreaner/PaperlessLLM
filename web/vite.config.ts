import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: false,
  },
  server: {
    proxy: {
      "/api": {
        target: "http://127.0.0.1:3000",
        changeOrigin: true,
        // The production dashboard and API share an origin. Mirror that in
        // development so the backend's localhost Host/Origin guard remains
        // exercised instead of being disabled for Vite's separate port.
        headers: { Origin: "http://127.0.0.1:3000" },
      },
      "/healthz": "http://127.0.0.1:3000",
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: "./src/test-setup.ts",
  },
});
