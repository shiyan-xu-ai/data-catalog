# Catalog frontend

The frontend is a React + Vite + TypeScript SPA served by `catalog-api` from
`CATALOG_WEBUI_DIR` (default `frontend/dist`). It shares the API's origin and
platform access boundary; no separate frontend host or application login is used.

## Supported workflow

- Register a table by composite id and optional owner.
- Browse registered tables and namespaces, with client-side search, sorting,
  grouping, and pagination on the table list.
- Inspect a table's version history and latest auxiliary datasets.
- Inspect a version's shape, byte breakdown, row/fragment/index counts, schema,
  Lance/writer metadata, and auxiliary datasets.
- View auxiliary schemas and bounded samples from supported Lance/Parquet data.
- Switch theme and local/UTC timestamp display.

Bucket overview, TTL policies and actions, protection controls, user recording,
and identity/role displays are not supported.
The API's enriched table listing returns the complete catalog; pagination in
the browser does not limit the server response.

## Implementation

- TanStack Router defines file-based routes and generates `src/routeTree.gen.ts`.
- TanStack Query handles fetching and cache invalidation.
- TanStack Table handles the table list.
- Zod response schemas in `src/lib/schemas.ts` mirror the Rust wire types.
- Tailwind and local shadcn/ui components supply styles and controls.

`src/lib/api.ts` contains the client for the retained REST endpoints.
Registration changes catalog metadata, not source data. Browsing reads the
catalog snapshot; it does not trigger a source-data sync.
See [README.md](../README.md#stats-freshness) for the pull model and freshness
limits.

## Development and verification

```sh
cd frontend
bun install --frozen-lockfile
bun --bun run dev
bun --bun run typecheck
bun --bun run test
bun --bun run build
```

Vite serves the local UI at port 5173 and proxies API calls to port 8080.
The build regenerates the route tree and emits static assets to `dist/`.
No frontend lint script is configured.
