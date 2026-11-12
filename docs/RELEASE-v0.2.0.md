# Release notes — v0.2.0

Each table now has a current version that the catalog records and caches, and
the table page shows much more of what Lance stores: the manifest and
transaction files behind each version, storage for the current version next to
all versions, per-fragment health, and per-column size.

## Current version and caching

- **Current version.** Each table row records a current version and its
  manifest e-tag. Every read that names no version answers for it.
  Registration sets it to the newest version.
- **Refresh.** `POST /catalog/v1/table/:id/refresh` advances the current
  version to the newest committed version and returns `previous_version` and
  `current_version`.
- **Cleanup and recreation.** If cleanup removes the current version, the next
  read advances to the newest one. A dataset recreated at the same location is
  detected by its manifest e-tag and advances too. A read that pins a version
  number never moves the current version.
- **Per-version cache.** Opened datasets, data statistics, fragment row counts,
  deleted-row counts, and the storage listing are cached per location, version,
  and e-tag, so a warm instance answers repeat reads without touching object
  storage. `CATALOG_CACHE_MEMORY_LIMIT_MB` (default 256) bounds the cache.

## API

- `GET /catalog/v1/table/:id/version/:version/files` returns the files that
  record a version, decoded from protobuf to JSON: the manifest file with its
  index section and inline transaction, plus a separate transaction file from
  older writers.
- `GET /catalog/v1/table/:id/fragment-measures` returns per-fragment data file
  count, data bytes, and deleted rows of the current version (min, p25,
  median, mean, p75, p99, max), read from the manifest alone.
- `GET /catalog/v1/table/:id/storage` adds a `current` breakdown by category:
  the objects the current version references (data files and their blob
  files, deletion files, index directories, its manifest and transaction
  file), next to every object under the location (`on_storage`).
- The Lance crates are pinned to exactly 14.0.2.

## UI

- **Overview.** An About card first (description, location, owner, registered
  and updated times, Lance file format, latest committed version), then rows,
  deleted rows, and the estimated monthly cost. Two storage tiles compare the
  current version with all versions. Fragment health lists per-fragment rows,
  data files, data size, and deleted rows, beside the share of small fragments.
- **Timeline and Versions.** Clicking a timeline entry or a version history row
  opens the same pane: commit, change, operation details, totals, writer and
  transaction, and a Source files view with the decoded manifest and
  transaction as a JSON tree. Tags and branches have their own tiles on the
  Versions tab; the current version and newer versions are marked.
- **Schema.** Schema-level metadata sits above the field table. The table shows
  each field's metadata count and a size-on-disk bar per column; clicking a
  field opens its details, including its size share and the raw Arrow field.
- **Layout.** The tab bar and the version, field, and index table headers stay
  pinned while the content scrolls. The header badges for format and version
  are gone; About shows both.

## Upgrade notes

- **Reads no longer follow new commits automatically.** A version committed
  directly to Lance becomes current only after a refresh or re-registration.
  Call the refresh route after writers commit, or read the version by number.
- Migrations `0003` and `0004` add `current_version` and `current_e_tag` to
  `tables`; they run at startup. Existing rows start without a recorded
  version, and their first read records the newest one.
