import { defineConfig } from "vite";

// Dev-server proxy: `/v1` and `/ext` are forwarded to the locally-running catalog-api
// (Tilt port-forwards the api pod's 8080 to localhost:8080). In production the built
// dist/ is served from S3+CloudFront and the api is reached at its public URL; set
// `VITE_API_BASE` to an absolute URL to override the default same-origin `/` base.
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
