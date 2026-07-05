import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { TanStackRouterVite } from "@tanstack/router-plugin/vite";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// Dev-server proxy: `/v1` and `/ext` are forwarded to a locally-running catalog-api on
// :8080. In production the built dist/ is served by catalog-api itself at the same origin,
// so the default `/` base needs no override; set `VITE_API_BASE` to an absolute URL only to
// point the dev server at a remote API.
export default defineConfig({
  plugins: [
    TanStackRouterVite({
      target: "react",
      routesDirectory: path.resolve(__dirname, "./src/routes"),
      generatedRouteTree: path.resolve(__dirname, "./src/routeTree.gen.ts"),
      autoCodeSplitting: true,
    }),
    react(),
  ],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    port: 5173,
    proxy: {
      "/v1": { target: "http://localhost:8080", changeOrigin: true },
      "/ext": { target: "http://localhost:8080", changeOrigin: true },
    },
  },
  build: {
    outDir: "dist",
    sourcemap: true,
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
