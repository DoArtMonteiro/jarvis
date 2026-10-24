import { defineConfig } from "vitest/config";

export default defineConfig({
  clearScreen: false,
  esbuild: { jsx: "automatic", jsxImportSource: "preact" },
  server: { port: 1420, strictPort: true, host: "localhost" },
  build: { target: "es2022", outDir: "dist", sourcemap: false },
  test: { environment: "jsdom", include: ["src/**/*.test.ts"] },
});
