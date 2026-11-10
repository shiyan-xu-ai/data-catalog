# Catalog frontend

The frontend is a React + Vite + TypeScript SPA served by `catalog-api` from
`CATALOG_WEBUI_DIR` (default `frontend/dist`). It shares the API's origin and
platform access boundary; no separate frontend host or application login is used.

## Supported workflow

- Browse the namespace tree from the root, with paginated child namespace and
  table lists.
- Create a namespace with optional properties, and drop an empty namespace
  (dropping a non-empty namespace reports an error).
- Register an existing Lance dataset by URI into a namespace, with an optional
  owner (`properties.owner`) and extra key/value properties.
- Inspect a table's location, properties, version, schema, and fragment and
  deleted-row stats, and deregister it.
- Switch between light, dark, and system themes.

Ids travel as `$`-joined path segments (root namespace `$`) in routes and API
paths, and display as dotted names. Names containing `$` are rejected.

## Implementation

- TanStack Router defines file-based routes and generates `src/routeTree.gen.ts`.
- TanStack Query handles fetching, `page_token` pagination, and cache invalidation.
- Zod schemas in `src/lib/schemas.ts` mirror the Lance REST Catalog spec models.
- Tailwind and local shadcn/ui components supply styles and controls.

`src/lib/api.ts` is the client for the supported `/v1/namespace/...` and
`/v1/table/...` spec routes and parses spec `{error, code}` error bodies.
`src/lib/ids.ts` builds ids from segment arrays. Registration and
deregistration change catalog metadata only, never the data at a table's
location.

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
