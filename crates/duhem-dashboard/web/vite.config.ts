/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Absolute path to ./src without pulling in @types/node (the tsconfig
// `types` allowlist excludes node): resolve it from the config URL.
const srcDir = new URL("./src", import.meta.url).pathname;

// base "./" keeps every asset reference relative, so the same bundle
// works served by `duhem-dashboard` at `/` and from a static export
// under any base path (#87).
export default defineConfig({
  base: "./",
  plugins: [react(), tailwindcss()],
  resolve: {
    // shadcn/ui convention: `@/` resolves to the SPA source root.
    alias: { "@": srcDir },
  },
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
