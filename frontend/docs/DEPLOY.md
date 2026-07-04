# Frontend deploy runbook

The catalog frontend is a static SPA (vanilla TS + Vite). `vite build` produces
`frontend/dist/` which is uploaded to S3 and served via CloudFront.

## Build

```sh
cd frontend
bun install        # or: npm install
bun run build      # or: npm run build  →  tsc --noEmit && vite build → dist/
```

`dist/` contains `index.html` + hashed JS/CSS assets. The SPA uses hash-based
routing (`#/`, `#/table/<id>`) so no server-side rewrites are required — any
deep link works from a static host.

## One-time S3 + CloudFront setup

1. **Create an S3 bucket** (e.g. `data-catalog-frontend`) with *Block Public
   Access* left ON (CloudFront OAI will be the only reader).
2. **Bucket policy** granting `s3:GetObject` to the CloudFront Origin Access
   Identity (OAI) you create in step 3 — AWS generates this for you in the
   CloudFront console ("Yes, use OAI"). Do NOT enable S3 static-website
   hosting (CloudFront serves the bucket via the REST API; S3 website hosting
   would make the bucket public).
3. **Create a CloudFront distribution**:
   - Origin: the S3 bucket, access via OAI.
   - Default cache behavior: viewer protocol policy `redirect-to-https`;
     allowed methods `GET, HEAD`; cache policy `CachingOptimized` (or a
     policy that respects `Cache-Control`); origin request policy `None`.
   - Default root object: `index.html`.
   - Response headers policy: add `Content-Type: text/html` for `*.html` is
     handled by S3 automatically; optionally add a `Content-Security-Policy`
     header that allows the SPA's origin and the catalog-api origin.
4. Note the distribution ID (e.g. `E1A2B3C4D5`) for the deploy script.

## Deploy

```sh
./frontend/scripts/deploy.sh <bucket-name> <distribution-id>
```

This runs `aws s3 sync --delete` (upload changed files, remove deleted) with a
5-minute browser cache, then invalidates `/*` on CloudFront so the new build
is live immediately.

## Wiring the frontend to the API

By default the SPA fetches `/v1/...` and `/ext/v1/...` from the same origin
the page was loaded from. In production the SPA is served from CloudFront and
the API is a separate Service — set `VITE_API_BASE` at build time to the
API's public URL:

```sh
VITE_API_BASE=https://catalog-api.example.com bun run build
```

This bakes the absolute base into the build. The Vite dev server
(`bun run dev`) proxies `/v1` and `/ext` to `http://localhost:8080` so a
locally-running catalog-api (e.g. via `tilt up`) is reachable from the dev
server without CORS configuration. (CORS is permissive on the API for GET in
v1.0.0 regardless; see `catalog-api/src/main.rs`.)

## Tilt integration

The local overlay (`deploy/overlays/local`) port-forwards the catalog-api pod's
port 8080 to `localhost:8080`. To run the frontend against the Tilt stack:

```sh
cd frontend && bun install && bun run dev
# then open http://localhost:5173
```

The Vite proxy forwards API calls to `localhost:8080`, which Tilt forwards to
the api pod. No extra Tilt resource is required for the frontend in v1.0.0
(the SPA is a dev-time convenience against the in-cluster api; production
serves static files from S3+CloudFront).
