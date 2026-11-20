# Frontend

A React, Vite, and TypeScript SPA. `catalog-api` serves it from
`CATALOG_WEBUI_DIR` at the API's origin, so it has no separate host, deploy, or
login. The UI only reads, apart from Refresh: namespaces and tables are created
and registered through the [API](../README.md#api).

## Layout

- **Sidebar.** Global search, the namespace tree, a Query entry, and theme (light, dark,
  system) and timezone (UTC, local) controls. It collapses to an icon rail; a
  click anywhere on the rail except those two controls expands it. The
  collapsed state, theme, and timezone persist in `localStorage`, and every
  timestamp renders in the chosen timezone. The Query page shows the rail
  without changing the stored state.
- **Namespace tree.** Loads children when a namespace opens; tables are leaves.
  A click anywhere on a namespace row toggles it (the chevron only shows the
  state); a click on its name also opens the namespace page. The current page's
  ancestors open automatically.
- **Search.** Matches namespace and table names. Tables come from
  `GET /v1/table`; namespaces from walking the tree on first focus (up to
  5,000). Focus it with Cmd/Ctrl+K or `/`.

Routes carry `$`-joined ids; names display dotted.

## Namespace page

Child namespaces, and a table list with name, business owner, technical owner,
and registered and updated times. Each row loads a cheap describe (Postgres
only) when it scrolls into view, at most six at a time, so a list never opens
a dataset.

## Table page

The Lance logo, the table name, a **Query this table** action, and six tabs:
Overview, Properties, Timeline, Versions, Schema, Indices. The active tab is in `?tab=`.

**State.** A tab mounts on first visit and stays mounted, and the eight most
recently visited tables stay mounted while hidden. Selected rows and panes,
expansion, and filters survive switching tabs or tables.
Opening a table through a link without `?tab=` restores its last tab; a URL that
names a tab opens that tab. Hidden pages ignore keyboard shortcuts and pause
video. Scroll position is not kept.

- **Overview.** An About card: table format (`Lance 2.2`), current version with
  its commit age, business and technical owner, and location. When a newer
  version is committed, it shows the latest version and a Refresh button that
  calls `POST /catalog/v1/table/:id/refresh`. Below: rows (`count_rows`), deleted
  rows, and the estimated monthly storage cost; storage by category for the current version
  and for every object; and fragment health (rows, data files, data size, and
  deleted rows per fragment, plus the share of fragments under Lance's
  1,048,576-row compaction target). The slow storage listing and stats load
  last.
- **Properties.** Table properties (owner, description, registration times, and
  any client keys except the two owners), Lance table metadata
  (`table_metadata`), and table config (`config`).
- **Timeline.** One row per retained version, newest first, 50 at a time,
  grouped under day headers, with the operation, version, tags, a one-line
  summary, and commit time. A dashed row marks versions removed by cleanup.
  Operation chips filter the list.
- **Versions.** Versions newest first, 50 at a time, under a pinned Trends chart
  of rows and data size. Tags and Branches tiles. Each row shows time, rows and
  change, data size, data files, and deletions; the current version and newer
  ones are badged.
- **Schema.** Schema-level metadata, then a field tree with type, nullability,
  description (the field's `description` metadata), and size on disk per
  top-level column. A raw JSON view is available.
- **Indices.** Name, type, columns, size, and coverage.

Clicking a timeline, version, or field row opens a side pane (Esc
closes it, arrow keys move). The pane shares the width evenly with its list on
wide screens and overlays it on narrow ones. Timeline and version panes show
the commit, changes against the next older loaded version, operation details,
totals, writer, transaction, and a Source files view of the decoded manifest
and transaction (`GET /catalog/v1/table/:id/version/:version/files`). Version,
field, and index tables pin their header and scroll sideways when wide.

## Query page

A top-level page (`/query`) that runs one read-only query at a time over any
registered tables (`POST /catalog/v1/sql`). While it is open the sidebar shows
as its icon rail; clicking the rail expands it until you leave the page. The
page stays mounted once visited, so tabs, text, results, and a running query
survive navigating away.

**Naming tables.** A query names a table by its full catalog path with every
segment double-quoted, for example `"a"."b"."events"`, and can join, union, or
nest tables freely. A plain path reads the table's current version;
`"a"."b"."events@v12"` reads version 12, and one query can mix versions. SQL
aliases work as usual, and an unaliased table is also reachable by its last
segment (`events.id`) unless two tables in the query share that name. A name
with one part is not a table.

**Workspace.** A left pane, a tab strip, an editor, and results.

- **Tables** (left pane). The namespace tree with a name box. A table's name
  inserts its quoted path at the cursor, and its chevron lists its columns with
  types; a column inserts its quoted name. The editor completes table paths,
  and column names once a table is expanded, inserted, or named in the query.
- **History** (left pane). The last 50 queries run in this browser, newest
  first; running the same text again moves it to the top. Selecting one loads it
  into the active tab without running it.
- **Tabs.** Open, switch, and close; each keeps its own text, results, and
  editor/results split. **Query this table** on a table page opens a new tab
  pre-filled with `SELECT * FROM "<path>" LIMIT 20` (`/query?table=<id>`).
- **Run.** Run or Ctrl/Cmd+Enter. Cancel stops waiting; the server's time limit
  ends the query.

Results show column types, nested values as JSON, binary as hex, the row count,
elapsed time, the table version when exactly one table was read (otherwise the
number of tables, with each `path@vN` in its tooltip), and a truncation notice.
Columns resize by dragging, and Download CSV exports the returned rows.

The row pane lists every column's value with a copy button and adds a
**Playback** tab for each playable value:

- **Lance blob columns.** A table field tagged `ARROW:extension:name:
  lance.blob.v2` whose children are `data`, `uri` (and optionally `position`,
  `size`) is a blob column. SQL returns it as the descriptor struct (`kind`,
  `position`, `size`, `blob_id`, `blob_uri`) with `lance-encoding:blob: true` in
  the column metadata, so aliases are recognized too. An external blob
  (`kind` 3, `blob_id` 0) whose `blob_uri` ends in a supported extension plays
  only `[position, position + size)`; a zero `size` plays from `position` to the
  end of the object. Other blob kinds are not played.
- **URL strings.** Any other value that is an http(s) or `s3://` URL ending in
  `.mp4`, `.m4v`, `.mov`, `.webm`, `.h265`, or `.hevc` plays as a whole object.

`s3://` objects play from a presigned URL
(`GET /catalog/v1/media/presign`), read directly from object storage.
Containers play in a `<video>` element; a blob range of a container is fetched
into memory first (up to 512 MiB). Raw H.265 streams are read in 2 MiB ranges,
decoded with WebCodecs where the browser supports H.265, and drawn on a canvas
at a chosen frame rate. The player has play/pause (click, Space, or K), a seek
bar, elapsed and estimated time, full screen (F), and 5-second skips (arrow
keys). Raw streams carry no timing, so a seek jumps to a byte position and times
are estimates. Download saves the file, or the blob's byte range.

## Code

- TanStack Router file routes (`src/routes/`, generating `src/routeTree.gen.ts`)
  and TanStack Query; query hooks and keys are in `src/lib/catalog-queries.ts`.
- `src/lib/api.ts` is the API client; `src/lib/schemas.ts` holds Zod schemas
  that mirror the spec and `/catalog/v1` responses.
- `src/lib/sql.ts` decides playability and quotes table paths; `src/lib/query-tabs.ts`
  and `src/lib/query-history.ts` hold the query tab and history state;
  `src/lib/hevc.ts` and `src/lib/hevc-player.ts` parse and play raw H.265.
- `src/lib/active.ts` tells components whether their page or tab is on screen.
- Tailwind with local shadcn/ui components; charts use Recharts.

## Development

```sh
cd frontend
bun install --frozen-lockfile
bun run dev         # http://localhost:5173, proxies /v1 and /catalog to :8080
bun run typecheck
bun run test
bun run build       # tsc --noEmit && vite build -> dist/
```

Set `VITE_API_BASE` to point the dev server at a remote API, which must allow
the cross-origin request. CI runs `bun run build` on frontend changes. No lint
script is configured.
