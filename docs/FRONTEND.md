# Catalog frontend

The frontend is a React + Vite + TypeScript SPA served by `catalog-api` from
`CATALOG_WEBUI_DIR` (default `frontend/dist`). It shares the API's origin and
platform access boundary; no separate frontend host or application login is used.

## Layout

A left sidebar holds global search and the namespace tree; the top bar shows a
breadcrumb and the theme toggle (light, dark, system).

- **Namespace tree.** Expands lazily, one list request per opened namespace,
  with tables as leaves. The current namespace or table is highlighted and its
  ancestors open automatically.
- **Global search.** Matches namespace and table names (not columns or
  properties). Tables come from `GET /v1/table`, which lists every table id;
  namespaces come from the lists the tree has loaded plus every table's
  ancestors. Focus it with ⌘K, Ctrl+K, or `/`.

## Pages

- **Namespace page.** Child namespaces and a table list with name, description,
  owner, location, and registered and updated times. Each row fills from a cheap
  describe (Postgres only) once it scrolls into view, at most six at a time, so
  listing never opens a dataset. Actions: create a namespace with optional
  properties, drop an empty namespace, and register an existing Lance dataset by
  URI with an optional owner, description, and extra properties.
- **Table page.** A header with the table name, format and version badges, and
  Deregister. A details sidebar always renders description, owner, location
  (copyable), storage format, current version, registered and updated times,
  remaining catalog properties, Lance table configuration, tags, and branches.
  The main area has four tabs, with the active tab in the `?tab=` query:
  - **Overview:** rows, data size, data files, deleted rows, version count, and
    index count; a per-column bytes chart (`GET /catalog/v1/table/{id}/column-storage`,
    requested only after the rest of the page loaded because it reads every
    fragment); a data and index storage donut, with deletion files shown as file
    and row counts because the manifest records no deletion byte size; and a
    rows and size growth chart over every version.
  - **Schema:** an expandable field tree with type, nullability, and field
    metadata, a raw JSON view, and schema-level metadata.
  - **Versions:** every version, newest first, paged 20 at a time, with
    timestamp, rows and row change, data size, data files, and deletions.
  - **Indices:** name, type, columns, size, and indexed versus unindexed rows.

Catalog-recorded facts come from the cheap describe, and Lance-derived sections
from the detailed describe and the version, index, tag, and branch routes. Each
Lance-derived section loads and fails on its own, so a table whose dataset is
missing still shows its sidebar facts. Empty sections render an explicit empty
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
