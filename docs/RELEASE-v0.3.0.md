# Release notes — v0.3.0

The table page gains a SQL tab with video playback, per-table state that
survives navigation, and owner fields; the namespace tree and lists were
reworked to match.

## SQL results and playback

- **SQL tab.** The read-only SQL panel has its own tab. Results scroll with the
  page, and clicking a row opens a pane with every column's type and full
  value, nested values as a JSON tree, and a copy button per value.
- **Video playback.** A result value that is an http(s) or `s3://` URL ending in
  `.mp4`, `.m4v`, `.mov`, `.webm`, `.h265`, or `.hevc` gets a Playback view.
  Container files play in a `<video>` element. Raw H.265 streams are read in
  byte ranges, decoded with WebCodecs, and drawn on a canvas with a seek bar,
  elapsed and estimated time, frame rate, full screen, and keyboard shortcuts.
  Download saves the file.
- **Lance blob columns.** A column tagged `lance.blob.v2` (queried as its
  descriptor) plays by the descriptor: an external blob with an absolute URI
  and a supported suffix plays only `[position, position + size)`. A zero size
  reads from `position` to the end of the object. Blobs stored inside the
  table's own files are not played yet.
- `POST /catalog/v1/table/:id/sql` now returns each result column's Arrow field
  metadata next to its name and type.

## API

- `GET /catalog/v1/media/presign?uri=s3://bucket/key` returns presigned https
  URLs (`url`, `download_url`, `expires_in_secs`) so the browser reads media
  from object storage directly; the catalog server never streams the bytes.
  `CATALOG_MEDIA_URL_TTL_MINUTES` (default 60) sets their lifetime.
- Concurrent refreshes of one table write its row once: refresh is a
  conditional `UPDATE` that only moves the current version forward, so a
  slower refresh never moves it back.

## Table page

- **State is kept.** Visited tables (the last eight) and opened tabs stay
  mounted, so selected rows and panes, SQL queries and results, expansion, and
  the open tab survive switching tables or tabs. Opening a table through a
  link without `?tab=` restores its last tab.
- **Overview.** The About card now shows table format (`Lance 2.2`), current
  version, business owner, technical owner, and location, each with a copy
  button where it helps. A Refresh button appears when a newer version is
  committed and makes it current.
- **Owners.** Business and technical owner are the `business_owner` and
  `technical_owner` table properties. Description, owner, and the registration
  times moved to the Properties tab.
- **Schema.** The field table shows each field's `description` metadata value.
- **Layout.** Detail panes (SQL row, schema field, version, timeline event)
  share one component and split the width evenly with their list. Version,
  field, and index tables scroll sideways with the header following. The
  Lance logo marks the table page.

## Navigation

- A click anywhere on a namespace row toggles it; the chevron only shows the
  state, and the name still opens the namespace page.
- The collapsed nav rail expands on a click anywhere except the theme and
  timezone buttons.
- Namespace table lists show business and technical owner, registered, and
  updated times; description, owner, and location are gone.

## Upgrade notes

- **No migrations.** The Postgres schema is unchanged.
- **Media access.** The presign route signs any `s3://` object the catalog's
  storage credentials can read; there is no prefix allowlist. Scope those
  credentials to what users should be able to read, and make sure each media
  bucket's CORS rules allow the UI's origin and the `Range` header.
- **Owners.** Set `business_owner` and `technical_owner` as table properties
  when registering. Re-register with `mode: overwrite` to change them; it
  replaces all properties, so resend the ones to keep.
