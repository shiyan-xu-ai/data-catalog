import { defineConfig } from "vite";

// Dev-server proxy: `/v1` and `/ext` are forwarded to a locally-running catalog-api on
// :8080. In production the built dist/ is served by catalog-api itself at the same origin,
// so the default `/` base needs no override; set `VITE_API_BASE` to an absolute URL only to
// point the dev server at a remote API.
export default defineConfig({
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
});
