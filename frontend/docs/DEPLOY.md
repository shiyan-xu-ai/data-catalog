# Frontend

The catalog frontend is a static SPA (vanilla TS + Vite). It is **built into the
`catalog-api` container and served same-origin** — there is no separate frontend
host or deploy step.

## How it ships

The repo-root `Dockerfile` has a `frontend` build stage that runs
`bun install --frozen-lockfile && bun run build`, producing `dist/` (an
`index.html` + hashed JS/CSS assets; hash-based routing, so any deep link works
without server rewrites). The runtime image copies `dist/` to `/app/webui` and
the server serves it at `/` (`CATALOG_WEBUI_DIR`). Because the SPA calls the API
at the same origin (default base `""`), no CORS configuration is needed. Deploy
is just deploying the container — see
[`docs/RUNBOOK-DEPLOY.md`](../../docs/RUNBOOK-DEPLOY.md).

## Local development

```sh
cd frontend
bun install
bun run dev        # http://localhost:5173, proxies /v1 and /ext to localhost:8080
```

Point it at a non-local API with `VITE_API_BASE=https://host bun run dev` (that
API must then allow the cross-origin request, which the same-origin container
deploy avoids).

## Build check

```sh
bun run build      # tsc --noEmit && vite build → dist/
```

The build typechecks against the hand-mirrored API shapes in `src/types.ts`, so
it catches drift from the serde-serialized responses. CI runs this on every PR.
