# Catalog frontend

The frontend is a React + Vite + TypeScript SPA served by `catalog-api` from
`CATALOG_WEBUI_DIR` (default `frontend/dist`). It shares the API's origin and
platform access boundary; no separate frontend host or application login is used.

## Layout

A left sidebar holds global search, the namespace tree, and, at the bottom, the
theme (light, dark, system) and timezone (UTC, local) controls. It collapses to
an icon rail (search, namespaces, theme, timezone). The collapsed state, theme,
and timezone persist in `localStorage`, and every absolute timestamp renders in
the chosen timezone. The top bar shows a breadcrumb. To the right of the
sidebar, page content scrolls on its own above an optional bottom dock that a
page can fill; the table page puts its SQL panel there.

- **Namespace tree.** Expands lazily, one list request per opened namespace,
  with tables as leaves. The current namespace or table is highlighted and its
  ancestors open automatically.
- **Global search.** Matches namespace and table names (not columns or
  properties). Tables come from `GET /v1/table`, which lists every table id;
  namespaces come from walking the namespace tree from the root on first focus
  (four list requests at a time, up to 5,000 namespaces), so empty namespaces
  are found too. Focus it with ⌘K, Ctrl+K, or `/`.

## Pages

- **Namespace page.** A header with the namespace name (its path is in the
  breadcrumb), child namespaces, and a table list with name, description,
  owner, location, and registered and updated times. Each row fills from a cheap
  describe (Postgres only) once it scrolls into view, at most six at a time, so
  listing never opens a dataset. The page is read-only: namespaces are
  created and dropped, and tables registered, through the API.
- **Table page.** A header with the table name and the storage format and
  served version badges (the namespace path is in the breadcrumb), six tabs
  (Overview, Properties, Timeline, Versions, Schema, Indices; the active tab is
  in the `?tab=` query; Schema and Indices show their counts), and a SQL panel
  docked at the bottom. The UI offers no deregistration.
  - **Overview:** six tiles: rows (with fragment count), current-version data
    (the manifest's data-file size and count), deleted rows, current index size,
    total bytes on storage, and the estimated monthly storage cost
    (`GET /catalog/v1/table/{id}/storage`). Each tile keeps a short caption; a
    (?) tooltip explains what the figure measures and excludes (for example
    blob payloads, index files kept by older versions, the rate and tier, and
    that the cost is an upper bound excluding requests and egress). Sections:
    - *Storage by column:* current-version inline column bytes per top-level
      column (`GET /catalog/v1/table/{id}/column-storage`); blob payload files
      and index files are excluded, as its (?) help explains.
    - *Storage composition:* every object under the location. With blob files,
      two donuts: blob files vs everything else, then the non-blob files by
      category (data, index, deletion, manifests, transactions, other) with
      shares of the non-blob total. Without blob files, one donut by category.
    - *Fragment distribution:* from the spec table stats
      (`POST /v1/table/{id}/stats`), horizontal bars of physical rows per
      fragment for min, p25, median, mean, p75, p99, and max, ordered by value
      with the mean in its own color (styled like Storage by column; hover a
      bar for its value), beside
      the share of fragments with fewer than 1,048,576 rows (Lance's default
      compaction target; fragments below it are compaction candidates). A
      table's `lance.compaction.target_rows_per_fragment` override is shown
      but not used, since the stats count against the fixed default.

    Column storage, the storage listing, and table stats are slow, so they are
    requested only after the rest of the page loaded; each shows its own
    skeleton and error.
  - **Properties:** the About facts first (description, owner, copyable
    location, registered and updated times, tags, branches), then client
    properties, Lance table metadata (`table_metadata`), and Lance table
    configuration (`config`).
  - **Timeline:** every retained version as a compact commit row, newest
    first, from `GET /catalog/v1/table/{id}/timeline`, 50 at a time with "Load
    earlier". Rows sit on a vertical rail under sticky `YYYY-MM-DD` day
    headers and show a color-coded operation marker and label (Compaction for
    `Rewrite`, Index built for `CreateIndex`, and so on), the version, its
    tags, one essential summary (row change, index name, first changed key,
    fragments before → after), and the commit time as `HH:mm:ss`; dates and
    times follow the timezone preference. A jump in version numbers shows a
    dashed row for the versions cleanup removed. Operation chips with counts
    over the loaded entries filter the list. Clicking a row (or Enter/Space)
    opens a detail pane beside the list (an overlay on narrow screens; Esc or
    the close button closes it, arrow keys move the selection) with the full
    ISO 8601 commit timestamp, operation, version and read version, tags, the
    row, data size, fragment, and deleted-row change against the next older
    loaded version ("—" for the oldest loaded one), all operation details
    (delete predicate, built indices, set and removed keys with values,
    compaction counts), the manifest totals at that version, the writer, and
    the transaction id and properties.
  - **Versions:** versions load newest first, 50 at a time
    (`POST /v1/table/{id}/version/list?descending=true`), with "Load earlier"
    for the next page; nothing reads every manifest up front. A Trends chart of
    rows and current-version data size covers exactly the loaded versions (its
    caption names the range, e.g. `v240–v289 (50 loaded)`) and stays pinned at
    the top while the version table scrolls under it, with the table header
    pinned just below. The table lists the loaded versions with timestamp,
    rows and row change against the next older loaded version ("—" for the
    oldest loaded one), data size, data files, and deletions. The served
    version carries a "current" badge; versions committed after it carry
    "newer than served" until a catalog refresh.
  - **Schema:** an expandable field tree with compact types (full type on
    hover), nullability, and field metadata, a raw JSON view, and schema-level
    metadata.
  - **Indices:** name, type, columns, size, and indexed versus unindexed rows.

  The **SQL panel** (labeled read-only) runs one query at a time against this
  table (`POST /catalog/v1/table/{id}/sql`). A version picker, listing the
  loaded versions newest first (its last option loads earlier ones; it shares
  the Versions tab's pages), sets which version the table name reads (the
  served version by default, labeled "Served (vN)"; newer versions are marked);
  any other version is addressable in SQL as `"<table>@v<N>"`, for
  example to compare versions. The panel opens with
  `SELECT * FROM "<table>" LIMIT 20`, runs on the Run button or Ctrl/Cmd+Enter,
  and shows column names and types, nested values as JSON, binary values as hex,
  row count, the queried version, elapsed time, a truncation notice past the
  server's row cap, and the server's error message. Cancel stops waiting; the
  server's time limit ends the query. The dock spans the content width with
  a collapsible editor on the left and scrollable results on the right (stacked
  on narrow screens). Drag the divider to resize the editor, or a column header
  edge to resize that column. Download CSV exports the returned rows, including
  full values; truncated results do not include rows beyond the server's cap.
  Dragging the dock's top edge resizes it. Clicking any empty part of its header
  bar, or the toggle button at the right end, collapses or expands it; the
  controls in the bar keep their own behavior. Height and collapsed state
  persist. Query results remain until the table page is closed
  or the user navigates away. The tabs stay usable while a query runs.

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
`table_metadata.*` / `config.*` keys from describe and version metadata. Registration and
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
