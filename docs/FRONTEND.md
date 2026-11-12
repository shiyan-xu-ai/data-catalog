# Catalog frontend

A React + Vite + TypeScript SPA that `catalog-api` serves from `CATALOG_WEBUI_DIR`
at the API's origin, behind the same platform access boundary; there is no
separate frontend host, deploy step, or login (see
[deployment](../ARCHITECTURE.md#deployment)). The UI is read-only: namespaces are
created and dropped, and tables registered and deregistered, through the
[API](../README.md#api).

## Layout

A left sidebar holds global search, the namespace tree, and, at the bottom, the
theme (light, dark, system) and timezone (UTC, local) controls. It collapses to
an icon rail (search, namespaces, theme, timezone); a click anywhere on the rail
expands it, except on the theme and timezone buttons. The collapsed state, theme,
and timezone persist in `localStorage`, and every absolute timestamp renders in
the chosen timezone. The top bar shows a breadcrumb. Page content scrolls on its
own.

- **Namespace tree.** Expands lazily, one list request per opened namespace,
  with tables as leaves. The current namespace or table is highlighted and its
  ancestors open automatically.
- **Global search.** Matches namespace and table names (not columns or
  properties). Tables come from `GET /v1/table`; namespaces come from walking the
  tree from the root on first focus (four list requests at a time, up to 5,000
  namespaces), so empty namespaces are found too. Focus it with ⌘K, Ctrl+K, or
  `/`.

Routes carry `$`-joined ids; names display dotted.

## Pages

- **Namespace page.** The namespace name (its path is in the breadcrumb), child
  namespaces, and a table list with name, description, owner, location, and
  registered and updated times. Each row fills from a cheap describe (Postgres
  only) once it scrolls into view, at most six at a time, so listing never opens
  a dataset.
- **Table page.** The Lance logo and the table name (namespace path in the breadcrumb) and seven
  tabs in a tab bar pinned to the top while content scrolls (Overview,
  Properties, Timeline, Versions, Schema, Indices, SQL; the active tab is in
  `?tab=`; Schema and Indices show their counts).
  - **Overview:** a full-width, untitled About card (description, owner,
    copyable location, registered and updated times, the current version's
    table format (`Lance <data storage version>`, e.g. `Lance 2.2`), and the
    latest committed version with its age, noting
    the current version when it is older), then three tiles: rows (with
    fragment count), deleted rows (with deletion file count), and the
    estimated monthly storage cost of every object under the location
    (`GET /catalog/v1/table/{id}/storage`). Tiles keep a short caption; tiles
    and sections each carry a one-line (?) tooltip instead of a subtitle.
    Sections:
    - *Storage: current version* (left) and *Storage: all versions* (right):
      the objects under the location that the current version references, and
      every object under it. With blob files, each shows two donuts (blob files
      vs everything else, then non-blob files by category: data, index,
      deletion, manifests, transactions, other); without, one donut by
      category. Legend rows show category and share.
    - *Fragment health:* per-fragment statistics (min, p25, median, mean, p75,
      p99, max), one row per measure: physical rows from
      `POST /v1/table/{id}/stats`, and data files, data size, and deleted rows
      from `GET /catalog/v1/table/{id}/fragment-measures` (each row loads,
      fails, or reads "Not recorded" on its own). Beside it, the share of
      fragments under 1,048,576 rows (Lance's default compaction target). A
      `lance.compaction.target_rows_per_fragment` override is shown but not
      used, since the stats count against the fixed default.

    The storage listing and table stats are slow, so they are requested only
    after the rest of the page loads; each has its own skeleton and error.
  - **Properties:** client properties, Lance table metadata
    (`table_metadata`), and Lance table configuration (`config`).
  - **Timeline:** every retained version as a compact commit row, newest
    first, from `GET /catalog/v1/table/{id}/timeline`, 50 at a time with "Load
    earlier". Rows sit on a vertical rail under sticky `YYYY-MM-DD` day headers
    and show a color-coded operation marker and label (Compaction for
    `Rewrite`, Index built for `CreateIndex`, and so on), the version, its
    tags, one essential summary (row change, index name, first changed key, or
    fragments before → after), and the commit time as `HH:mm:ss`. A jump in
    version numbers shows a dashed row for the versions cleanup removed.
    Operation chips with counts over the loaded entries filter the list.
    Clicking a row (or Enter/Space) opens a detail pane beside the list (an
    overlay on narrow screens; Esc or the close button closes it, arrow keys
    move the selection) with the full ISO 8601 commit timestamp, operation,
    version and read version, tags, the row, data size, fragment, and
    deleted-row change against the next older loaded version ("—" for the
    oldest loaded one), all operation details (delete predicate, built indices,
    set and removed keys with values, compaction counts), the manifest totals
    at that version, the writer, and the transaction id and properties. The
    pane's Source files view shows the files the entry was read from
    (`GET /catalog/v1/table/{id}/version/{version}/files`): the manifest file
    and, from older writers, the transaction file, each section as a
    collapsible JSON tree with a copy button.
  - **Versions:** newest first, 50 at a time
    (`POST /v1/table/{id}/version/list?descending=true`) with "Load earlier";
    nothing reads every manifest up front. A Trends chart of rows and
    current-version data size covers exactly the loaded versions and stays
    pinned at the top while the version table scrolls under it. Below Trends,
    Tags (name → version) and Branches (name, parent branch and version,
    manifest size) tiles. The table lists timestamp, rows and row change
    against the next older loaded version ("—" for the oldest loaded one), data
    size, data files, and deletions. The current version carries a "current"
    badge; later versions carry "newer than current" until a catalog refresh.
    Clicking a row opens the Timeline's pane below the Trends chart, with the
    version's event fetched as a one-event page
    (`?page_token=<version + 1>&limit=1`) and changes measured against the next
    older loaded version.
  - **Schema:** schema-level metadata on top, then an expandable field tree
    with compact types (full type on hover), nullability, a count of field
    metadata entries, and size on disk (last), plus a raw JSON view. Size on
    disk is a bar per top-level column scaled to the largest column, with the
    size inside it, from `GET /catalog/v1/table/{id}/column-storage` (blob
    payloads and index files excluded); it loads on its own when the tab opens.
    Clicking a row (or Enter/Space; Esc closes, arrows move) opens a field pane:
    path, full type, nullability, size with its share of all columns and rank,
    field metadata, the schema-level entries keyed `<key>:<column>` to that
    column (such as `blob-codec:<column>`), and the Arrow field as a JSON tree.
  - **Indices:** name, type, columns, size, and indexed versus unindexed rows.

  The version, field, and index tables pin their header row below the tab bar
  (the version table's below the Trends chart) and scroll sideways with the
  header following when wider than their column.

  The timeline, version, field, and SQL row panes are one shared side pane.
  On wide screens it and its list split the width evenly, with every list
  column kept; on narrow screens it opens as an overlay.

  The **SQL** tab (labeled read-only) runs one query at a time against this
  table (`POST /catalog/v1/table/{id}/sql`). A version picker lists the loaded
  versions newest first (its last option loads earlier ones; it shares the
  Versions tab's pages) and sets which version the table name reads: the
  current version by default, labeled "Current (vN)", with newer versions
  marked. Any other version is addressable as `"<table>@v<N>"`. The panel opens
  with `SELECT * FROM "<table>" LIMIT 20` and runs on the Run button or
  Ctrl/Cmd+Enter. Results show column names and types, nested values as JSON,
  binary values as hex, row count, the queried version, elapsed time, a
  truncation notice past the server's row cap, and the server's error message.
  Cancel stops waiting; the server's time limit ends the query. The results
  scroll with the page like the other tables: the toolbar and the result
  header pin below the tab bar, and wide results scroll sideways with the header
  following. Side by side (wide screens), the collapsible editor stays pinned on
  the left at the viewport's height; hidden, the results take the full width;
  stacked (narrow screens), the editor sits above them. Drag the divider to
  resize the editor, or a column header edge to resize that column. Download
  CSV exports the returned rows with full values; truncated results omit rows
  past the server's cap. Clicking a result row (or Enter/Space; Esc closes,
  arrows move) opens a row pane beside the results like the schema field pane:
  every column's name, type, and full value, nested values as a JSON tree, each
  with a copy button; a new run clears the selection. When some value in the
  row is an http(s) or `s3://` URL ending in a video extension (`.mp4`,
  `.m4v`, `.mov`, `.webm`, `.h265`, `.hevc`; a query string is allowed), the
  pane adds a Playback tab with a player per such column and a Download button
  (for `s3://` objects, a presigned URL served as an attachment).
  `s3://` objects play from a presigned https URL
  (`GET /catalog/v1/media/presign?uri=`), so the browser reads them from object
  storage directly; the bucket's CORS rules must allow the UI's origin and the
  `Range` header. Container formats play in a `<video>` element. Raw H.265
  streams (`.h265`, `.hevc`) are read in 2 MiB byte ranges, split into
  pictures, decoded with WebCodecs (where the browser and hardware support
  H.265; otherwise the pane says so and stops reading), and drawn on a canvas
  at a chosen frame rate, since raw streams carry no timing; reading pauses
  while 30 frames are buffered. Its controls: play/pause (also a click on the
  video, Space, or K), a seek bar showing the read range, elapsed and
  estimated total time, the frame rate, and full screen (F); arrow keys skip
  5 seconds. A seek jumps to that byte fraction of the file and resumes at the
  next random access picture, so times are estimates from the average picture
  size read so far. The
  editor loads when the tab first opens and then stays mounted, so the query and
  results survive switching tabs until the user leaves the table page; the other
  tabs stay usable while a query runs.

Catalog-recorded facts come from the cheap describe; Lance-derived sections come
from the detailed describe and the version, index, tag, and branch routes. Each
Lance-derived section loads and fails on its own, so a table whose dataset is
missing still shows its About facts. Empty sections render an explicit empty
state.

## Implementation

- TanStack Router defines file-based routes and generates `src/routeTree.gen.ts`.
- TanStack Query handles fetching, `page_token` pagination, and cache
  invalidation; query hooks and keys live in `src/lib/catalog-queries.ts`.
- `src/lib/api.ts` is the client for the spec routes and the `/catalog/v1` read
  and SQL routes, and parses `{error, code}` error bodies.
- Zod schemas in `src/lib/schemas.ts` hand-mirror the spec models and the
  `/catalog/v1` responses; `tsc` checks them against their callers.
- `src/lib/ids.ts` builds ids from segment arrays; `src/lib/table-metadata.ts`
  reads the manifest summary and `table_metadata.*` / `config.*` keys from
  describe and version metadata.
- Tailwind and local shadcn/ui components supply styles and controls; charts use
  Recharts through the shadcn chart pattern (`src/components/ui/chart.tsx`).

## Development

```sh
cd frontend
bun install --frozen-lockfile
bun --bun run dev
bun --bun run typecheck
bun --bun run test
bun --bun run build
```

The dev server listens on port 5173 and proxies `/v1` and `/catalog` to a
`catalog-api` on port 8080. To use a remote API instead, set `VITE_API_BASE`
(for example `VITE_API_BASE=https://host bun --bun run dev`); that API must allow
the cross-origin request. The build runs `tsc --noEmit && vite build`,
regenerating the route tree and emitting static assets to `dist/`. CI runs the
build on frontend changes. No lint script is configured.
