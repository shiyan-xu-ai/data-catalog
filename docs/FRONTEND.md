# Catalog frontend

The frontend is a React + Vite + TypeScript SPA served by `catalog-api` from
`CATALOG_WEBUI_DIR` (default `frontend/dist`). It shares the API's origin and
platform access boundary; no separate frontend host or application login is used.

## Layout

A left sidebar holds global search, the namespace tree, and, at the bottom, the
theme (light, dark, system) and timezone (UTC, local) controls. It collapses to
an icon rail (search, namespaces, theme, timezone). The collapsed state, theme,
and timezone persist in `localStorage`, and every absolute timestamp renders in
the chosen timezone. The top bar shows a breadcrumb.

- **Namespace tree.** Expands lazily, one list request per opened namespace,
  with tables as leaves. The current namespace or table is highlighted and its
  ancestors open automatically.
- **Global search.** Matches namespace and table names (not columns or
  properties). Tables come from `GET /v1/table`, which lists every table id;
  namespaces come from walking the namespace tree from the root on first focus
  (four list requests at a time, up to 5,000 namespaces), so empty namespaces
  are found too. Focus it with ⌘K, Ctrl+K, or `/`.

## Pages

- **Namespace page.** Child namespaces and a table list with name, description,
  owner, location, and registered and updated times. Each row fills from a cheap
  describe (Postgres only) once it scrolls into view, at most six at a time, so
  listing never opens a dataset. Actions: create a namespace with optional
  properties, drop an empty namespace, and register an existing Lance dataset by
  URI with an optional owner, description, and extra properties.
- **Table page.** A header with the table name and format and version badges,
  five tabs on the left (the active tab is in the `?tab=` query), and a SQL
  panel on the right. The UI offers no deregistration; the API still does.
  - **Overview:** rows; current-version data (the manifest's data-file size,
    which excludes blob payloads, index files, and files kept only by older
    versions); total bytes on storage and the estimated monthly storage cost
    with its rate and tier (`GET /catalog/v1/table/{id}/storage`); data files,
    deleted rows, version count, and index count. Charts: current-version bytes
    per column (`GET /catalog/v1/table/{id}/column-storage`), the on-storage
    composition by file category, and rows and data size over every version.
    Both catalog routes are slow, so they are requested only after the rest of
    the page loaded and show skeletons until then.
  - **Properties:** the About facts first (description, owner, copyable
    location, storage format, current version, registered and updated times,
    tags, branches), then client properties, Lance table configuration, and
    schema-level metadata.
  - **Schema:** an expandable field tree with compact types (full type on
    hover), nullability, and field metadata, a raw JSON view, and schema-level
    metadata.
  - **Versions:** every version, newest first, paged 20 at a time, with
    timestamp, rows and row change, data size, data files, and deletions.
  - **Indices:** name, type, columns, size, and indexed versus unindexed rows.

  The **SQL panel** runs one read-only DataFusion query at a time against the
  table's current version (`POST /catalog/v1/table/{id}/sql`). It opens with
  `SELECT * FROM "<table>" LIMIT 20`, runs on the Run button or Ctrl/Cmd+Enter,
  and shows column names and types, compact nested and binary values, row
  count, elapsed time, a truncation notice past the server's row cap, and the
  DataFusion error message. Cancel stops waiting; the server's time limit ends
  the query. The panel collapses and widens by dragging its left edge; both
  persist. The tabs stay usable while a query runs.

Catalog-recorded facts come from the cheap describe, and Lance-derived sections
from the detailed describe and the version, index, tag, and branch routes. Each
Lance-derived section loads and fails on its own, so a table whose dataset is
missing still shows its About facts. Empty sections render an explicit empty
state.

Ids travel as `$`-joined path segments (root namespace `$`) in routes and API
paths, and display as dotted names. Names containing `$` are rejected.

## Implementation

- TanStack Router defines file-based routes and generates `src/routeTree.gen.ts`.
- TanStack Query handles fetching, `page_token` pagination, and cache
  invalidation; query hooks and keys live in `src/lib/catalog-queries.ts`.
- Zod schemas in `src/lib/schemas.ts` mirror the Lance REST Catalog spec models
  and the catalog's column-storage response.
- Tailwind and local shadcn/ui components supply styles and controls; charts use
  Recharts through the shadcn chart pattern (`src/components/ui/chart.tsx`).

`src/lib/api.ts` is the client for the supported `/v1/namespace/...` and
`/v1/table/...` spec routes and the `/catalog/v1` column-storage route, and
parses `{error, code}` error bodies. `src/lib/ids.ts` builds ids from segment
arrays, and `src/lib/table-metadata.ts` reads the Lance manifest summary and
`config.*` keys from describe and version metadata. Registration and
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

Vite serves the local UI at port 5173 and proxies `/v1` and `/catalog` to port 8080.
The build regenerates the route tree and emits static assets to `dist/`.
No frontend lint script is configured.
