/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// base "./" keeps every asset reference relative, so the same bundle
// works served by `duhem-dashboard` at `/` and from a static export
// under any base path (#87).
export default defineConfig({
  base: "./",
  plugins: [react()],
  server: {
    // Local dev against a running `duhem-dashboard` (default port).
    proxy: {
      "/api": "http://127.0.0.1:7878",
    },
  },
  test: {
    environment: "jsdom",
    globals: false,
  },
});
