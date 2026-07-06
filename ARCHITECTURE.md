# Architecture (as-built)

This document describes the system as implemented. The broader long-term vision
lives in [`docs/design.md`](docs/design.md).

## Overview

One binary, `catalog-api`, runs as a single stateless container on Cloud Run. It
serves the REST API and the SPA, and runs the S3 sync and the TTL engine. There
is no leader election, no lock, and no database: **S3 is the sole store**. Any
instance can serve any request. The sync runs request-scoped, triggered by
Cloud Scheduler — Cloud Run throttles CPU between requests, so a background loop
would freeze while idle.

```
Cloud Scheduler (cron) ─▶ POST /internal/jobs/sync ─┐
                                                      │
Browser ─ IAP/Trident ─▶ catalog-api (Cloud Run, axum)
                          ├─ REST /v1 + /ext  (merged read model)
                          ├─ static SPA       (same origin, no CORS)
                          └─ sync + TTL      ◀────────┘
                                    │
                                    ▼  S3 (sole store)
  <table>/<ts>/...            immutable version dirs; LIST/open; TTL deletes
  _catalog/registry          derived snapshot (Lance), sync-written, last-wins
  _catalog/meta/<id>.json    authored overlay {owner, ttl_policy, protected[],
                             deleting[]} — ETag-CAS mutations
  _catalog/ttl_audit         append-only audit log (Lance)
  _catalog/storage_scan      bucket-storage breakdown (Lance), sync-tail-written
  _catalog/users             recorded users (Lance), merge_insert-upserted
```

## Regional deployment

The service is regional: one deployment per region, colocated with that
region's target buckets. The region, the target buckets, and each bucket's
registered namespaces are declared in `catalog-config.yaml` (committed,
baked into the image). Table ids are region-first composites
(`<region>:<bucket>:<namespace>:<name>`), so a future multi-region setup —
one deployment per region — can aggregate catalogs without id collisions.
Catalog state stays in the dedicated catalog bucket, never in target buckets.

## State model: derived vs authored

Each catalog version is one **immutable** timestamp directory
(`<table>/<ts>/dataset.lance/...`), never rewritten after export. That
immutability splits registry state in two:

1. **Derived state** — versions, shapes, byte splits, row counts, schemas, aux
   entries. A pure function of immutable S3 content, so any full sync
   regenerates it. The derived snapshot (`_catalog/registry`, a Lance dataset)
   is therefore written **whole, last-wins, with no coordination**: two
   concurrent syncs, or a Scheduler double-fire, cannot corrupt it — the worst
   case is bounded staleness until the next sync. Lost-update protection is
   meaningless for recomputable data.
2. **Authored state** — per-table `owner` + `ttl_policy`, per-version
   `protected`, plus transient `deleting` markers. The only human-mutated data,
   a few hundred bytes per table, stored as one JSON object per table at
   `_catalog/meta/<table_id>.json`. Mutations are object-store conditional
   writes (ETag compare-and-set), so a per-table object is its own conflict
   domain and S3 itself serializes concurrent edits.

The overlay also **defines the registered set**: the catalog is a curated
allowlist, not a mirror of the bucket. A table is registered iff an overlay
object exists for it (created by `DeclareTable`), and only registered tables are
synced. This is what lets a whole registered namespace serve a scoped catalog —
an operator registers the handful of tables to track, and unregistered tables
on S3 are never touched.

Everything the old k8s leader + write-lock protected was the *entanglement* of
these two in one read-modify-write blob. Separating them dissolves the
coordination problem instead of porting it. A read merges the two back into the
`TableEntry` wire shape the REST API and frontend already expect
(`catalog-api/src/catalog.rs`): the snapshot supplies derived fields, the
overlay supplies `owner`/`ttl_policy`/`protected`; a table registered but not yet
synced (declared before its first sync) is materialized as a stub.

### The authored overlay (`MetaStore`)

`catalog-store/src/overlay.rs` is the CAS primitive. `mutate_meta(id, f)` reads
the overlay object (or starts fresh if absent), applies the caller's closure,
then writes with `PutMode::Update` pinned to the observed ETag (or
`PutMode::Create` if it was absent). If another writer committed in between, the
store returns `Precondition`/`AlreadyExists`; the read-modify-write retries on
the fresh state, bounded by a retry ceiling. At the catalog's admin write volume
contention is effectively never, but the retry makes it correct regardless.

> `object_store`'s `LocalFileSystem` does not implement `PutMode::Update`, so the
> overlay must be backed by S3 / a MinIO-compatible endpoint (production, local
> dev) or `InMemory` (tests). The sync/data path still uses `LocalFileSystem`
> freely — it does no conditional writes.

## The sync

`catalog-store/src/sync.rs` classifies one table's directory
(`sync_table`). `catalog-api/src/sync.rs` wraps it into one request-scoped
pass: `run_sync` reads the registered set (the overlay objects), re-derives each
registered table's version set from S3, writes the whole snapshot as exactly that
set (last-wins), reconciles stale overlay markers, and best-effort prunes old
registry manifest versions. It never enumerates the root, so unregistered tables
cost nothing and never appear.

### Multi-bucket sync targets

`catalog-config.yaml` declares the buckets and, per bucket, the registered
namespace prefixes; there is no discovery of namespaces from S3 — a namespace
becomes syncable only by being listed in the config. Each `(bucket, namespace)`
pair becomes one `SyncTarget`, sharing one object store per bucket. Discovery
per target starts with a delimiter LIST of the namespace's own directory to
find its table dirs; the storage-analysis tail (below) separately
delimiter-LISTs each bucket's root and partitions what it finds into
registered (a top-level prefix that is an exact registered namespace) versus
unexplored (everything else — sibling prefixes, ancestor levels of a nested
namespace, and loose root objects). All targets' pending versions — across
every bucket and namespace — feed the **same global `buffer_unordered` version
queue** described below, so a large namespace in one bucket cannot starve a
small namespace in another.

### Execution shape (performance)

Syncing is S3-latency-bound, so the implementation minimizes round-trips and
maximizes useful concurrency:

- **One recursive LIST per version.** A version's shape, byte splits (lance-core/
  sidecar/segments/other-aux), aux entries, format detection, and fingerprint are
  all derived in memory from a single recursive LIST of its timestamp directory
  (`catalog-store/src/format.rs` helpers are pure functions over that object
  list). The only other per-version IO is opening the main Lance dataset.
- **Manifest-first Lance stats.** The one manifest the open fetches is milked for
  everything it carries: row count (Σ per-fragment counts — `count_rows()`, which
  can read deletion files, runs only as a fallback when a fragment's count is
  unknown), fragment count, schema, the lance manifest version, and the writer
  version. `load_indices` (extra index-metadata IO) is gated by
  `CATALOG_SYNC_DEEP_STATS` (`all`/`latest`/`none`); the open itself always runs,
  so shape/`partial` classification — and therefore TTL safety semantics — are
  identical in every mode. The sync report breaks out cumulative LIST time vs
  Lance-open time (`list_secs`/`open_secs`/`objects_listed`), so a slow table
  shows *why* it is slow.
- **The unit of work is the version, not the table.** All registered tables'
  version dirs are discovered first (one delimiter LIST per table, concurrently),
  then every pending version from every table feeds one global
  `buffer_unordered(CATALOG_SYNC_CONCURRENCY)` queue — so a 500-version table
  interleaves with 3-version tables instead of serializing behind them, and wall
  time ≈ `pending versions / concurrency`.
- **Carry-forward.** Versions are immutable timestamp dirs, so a version already
  in the prior snapshot with a clean (non-partial) classification is copied
  forward with zero S3 traffic. Only new versions, `partial` versions (re-synced to
  self-heal), and `deleting`-marked versions (a failed TTL delete may have removed
  part of the prefix — carrying would freeze stale byte counts) are actually
  synced. Steady-state syncs cost O(new versions), and an interrupted first sync
  resumes instead of restarting.
- **Incremental publication.** As each table's last pending version lands, the
  snapshot is rewritten (throttled to a few seconds apart, plus a final
  unconditional write) — a long first sync fills the catalog table-by-table
  rather than all-or-nothing, and a crash/timeout keeps everything completed so
  far.

Because the snapshot is rebuilt from the registered set each pass, it stays an
allowlist automatically: a deregistered table (overlay deleted) is simply not
synced and drops out of the snapshot, and within a table, a version no longer on
S3 (removed out of band, or by a TTL apply) is gone from the freshly-derived set.
A registered table with no directory on S3 yet syncs cleanly to zero versions (a
stub) until data lands.

A single registered table failing to sync is isolated — logged, counted, and
kept from the prior snapshot — rather than aborting the whole cycle and stalling
every other table's freshness. A version transiently seen as `partial` (e.g. its
dataset momentarily failed to open) is re-derived cleanly by a later sync, so it
does not stay permanently partial; a transient partial re-observation shows for
one cycle and self-heals, and the shape safety gate blocks TTL during that window
(the safe direction).

**Overlay reconciliation.** After writing the snapshot, the sync prunes overlay
`protected`/`deleting` entries whose version id is no longer in the snapshot —
clearing the tombstone a TTL apply leaves behind once the version is truly gone
(see [TTL engine](#ttl-engine)). It never touches `owner`/`ttl_policy`.

**Idempotent under double-fire.** The `/internal/jobs/sync` endpoint serializes
syncs on a single instance with an in-process mutex (so a Scheduler double-fire
doesn't run two at once on the same instance); cross-instance concurrency is safe
regardless because the snapshot is last-wins.

### Version discovery

For each registered table the sync lists that table's directory
(`<root>/<table_id>/`) for timestamp subdirectories. A timestamp directory name
is parsed as `YYYY-MM-DD_HH-MM-SS` or `YYYY-MM-DD-HH-MM-SS` (name variance is
handled), normalized to an ISO8601 version id. One timestamp directory = one
catalog version. Every timestamp version from earliest to latest is supported,
regardless of pre/post cutoff layout.

### Shape classification

Each timestamp directory is classified into one of five shapes:

- `full` — a main lance directory (openable as a Lance dataset) + `segments/`
  [+ `dataset.sidecar/`].
- `lance_only` — main lance present, no `segments/`.
- `seg_only` — `segments/` only, no main lance (partial; cannot open, aux only).
- `lance_only_partial` — the apparent main lance directory contains only nested
  lances (`index_datasets/`, `tag_datasets/`) with no `_versions/`+
  `_transactions/` at its own root (partial).
- `empty` — no recognized directories.

A `partial` flag is set on any version whose shape is not `full`.

### Main lance detection

The main lance directory is detected by the presence of `_versions/` and
`_transactions/` at the directory root, never by name. The name varies in
practice (`dataset.lance/` for most tables, `dataset/` for some). If a candidate
directory lacks `_versions/`+`_transactions/` at its root and contains only
nested lances, the version is `lance_only_partial`.

### Size split

Storage bytes are attributed per-component so that pre-cutoff versions (where
sidecar directories live inside `dataset.lance/` and inflate its size) are
correctly decomposed:

- `lance_core_bytes` — `_versions/`, `_transactions/`, `_indices/`, `data/`.
- `sidecar_bytes` — sidecar directories, wherever located (top-level
  `dataset.sidecar/` if present, else the known sidecar subdirs inside the main
  lance dir).
- `segments_bytes` — the `segments/` directory.
- `other_aux_bytes` — any other top-level aux directory not accounted for above.

`storage_bytes_total` is the logical (deduped) sum of these components. See the
[TTL runbook](docs/RUNBOOK-TTL.md) for the logical-vs-physical distinction that
matters for `reclaimable_bytes`.

### Auxiliary tables (sidecar / nested sidecar)

Aux entries carry a placement taxonomy: **`sidecar`** for top-level aux
(`dataset.sidecar/`, `segments/`, known top dirs) and **`nested_sidecar`** for
aux living inside the main lance dir. Every nested lance dataset root (a dir
with `_versions/`, at any depth — e.g.
`dataset.lance/tag_datasets/<n>.lance/.../segment_tags.lance`) is discovered
from the version's single object LIST at zero extra request cost: an existing
aux entry whose path is a root is enriched in place, and deeper roots become
their own entries named by relative path, with `dataset_path` pointing at the
openable root. Lance aux entries get manifest-derived `row_count`/schema/writer
stats (deep-stats-gated, manifest-only — no index loads). A version whose main
lance dir holds only nested datasets stays `partial` (primary missing). The
sample endpoint reads rows through `dataset_path` (lance) or the entry path
(parquet, via DataFusion) — strictly read-only, clamped, and timeout-bounded.

### Aux format detection

Aux directories (`segments/`, `dataset.sidecar/`, `curated_csv/`, `row_counts/`,
nested lances, etc.) are recorded as `AuxEntry`s with a detected `format`:

- `parquet` — `_SUCCESS` + `part-*.parquet`.
- `lance` — `_versions/` + `_transactions/`.
- `csv` — `*.csv`.
- `mixed` / `unknown` — otherwise, with a fingerprint via LIST.

Aux names are not hardcoded beyond the sidecar/segments/lance-core sets; the dir
name is recorded as the `role`.

### Pre+post 2026-06-26 cutoff aux layout

A layout transition occurred around 2026-06-26:

- **Pre-cutoff** (< 2026-06-26): sidecar directories live *inside*
  `dataset.lance/` (alongside lance-core directories). The sync reads them from
  there.
- **Post-cutoff** (>= 2026-06-26): sidecar directories move to a top-level
  `dataset.sidecar/`; `dataset.lance/` is clean (lance-core only).
- **Transition** (~2026-06-12): sidecar content is duplicated both inside
  `dataset.lance/` and at top-level `dataset.sidecar/` (dual-write). The sync
  deduplicates: when a top-level `dataset.sidecar/` is present it is preferred;
  otherwise sidecar bytes are read from inside the main lance dir.

The sync supports both layouts so the full version history is cataloged
correctly. The real-world layout is captured in
[`docs/design.md`](docs/design.md), including concrete annotated example tables.

## The registry (derived snapshot)

The registry is a Lance dataset under `_catalog/registry` (path configurable via
`CATALOG_REGISTRY_PATH`). It stores one derived `TableEntry` per table: id
(the region:bucket:namespace:name composite), name, region, bucket, namespace,
root_location, last_synced, versions[], and aux_latest[] — plus
`owner`/`ttl_policy`/per-version `protected` slots that the sync leaves empty
and the overlay fills in at read time. Each `TableVersion` carries version_id,
timestamp, snapshot_path, shape, partial, protected, the per-component byte
totals, object_count, row_count, num_fragments, schema_json, num_indices,
aux[], and synced_at.

The persisted registry types are forward/backward compatible (absent fields
default), so a schema addition does not break an older reader or a snapshot
written by an older writer. The sync tail-calls Lance `cleanup_old_versions` on
the registry so its manifest history does not grow unbounded (each sync adds one
version).

Registry reads fail closed: `read_registry` returns `Ok(None)` only for a
genuinely absent dataset (first boot); any other read failure is an `Err`.

### Storage analysis (`_catalog/storage_scan`)

After each sync writes the registry snapshot, a storage-analysis tail step
(`storage_tail` in `catalog-api/src/sync.rs`) walks each configured bucket's
top-level layout with delimiter LISTs (never a full recursive enumeration) and
writes one `StoragePrefixStat` row per top-level prefix (plus a `(root)`
pseudo-row for loose root objects, and each ancestor level's siblings for a
nested registered namespace) to `_catalog/storage_scan` (path configurable via
`CATALOG_STORAGE_SCAN_PATH`). A prefix that exactly matches a registered
namespace is sized by aggregating that namespace's just-written registry
entries (bytes, objects, table count); every other prefix is reported
unexplored (name-only, no size). `GET /ext/v1/storage` serves the latest scan
as-is; the tail overwrites the whole dataset each run (a point-in-time
snapshot, not a durable history) and best-effort prunes old manifest versions
the same way the registry does. A tail failure is logged and does not fail the
sync.

### Users (`_catalog/users`)

`_catalog/users` (path configurable via `CATALOG_USERS_PATH`) records every
caller email the identity middleware has seen, upserted with Lance
`merge_insert` keyed on `email` so a repeat sighting updates `last_seen_at` in
place instead of duplicating the row. Identity comes from the IAP
`x-goog-authenticated-user-email` header (`catalog-api/src/identity.rs`);
`GET /ext/v1/users` and `GET /ext/v1/me` overlay each user's `role` from
`catalog-config.yaml`'s `admins` list at read time, so the config file remains
the single source of truth for the admin role and a demotion takes effect
immediately without rewriting stored rows.

## The read model (merged view)

`catalog-api/src/catalog.rs` holds the merged read model, shared across handlers.
It reads the snapshot + all overlays and merges them into the `TableEntry` wire
shape. Because Cloud Run throttles CPU between requests, there is no background
refresh loop: the view is revalidated **lazily** — re-read from storage when
older than `CATALOG_CACHE_TTL_SECS` (default 5s), single-flighted so a burst of
stale reads triggers one reload — and immediately after a mutation invalidates it
(read-your-writes on the same instance). On a revalidation error the last good
view is kept and served; staleness is bounded by the sync cadence anyway.
`/readyz` reports ready once the view has loaded at least once.

Mutations re-fetch the overlay fresh inside every write (the cache is for reads
only): a handler calls `MetaStore::mutate_meta`, then invalidates the read cache.

## Startup and shutdown

At startup the process fetches AWS credentials from Secret Manager (see
[Credentials](#credentials)), builds the sync + overlay object stores, and binds
one HTTP server on the single port (`PORT` on Cloud Run, else
`CATALOG_BIND_ADDR`). The server drains in-flight requests on `SIGTERM`/Ctrl-C via
graceful shutdown before the process exits. Duration config values (`*_SECS`) are
validated at load: a set-but-unparseable value or `0` is a hard startup error,
not a silent fallback.

## Credentials

Cloud Run runs under a *GCP* service account, but the catalog's data is in *AWS*
S3, so there is no ambient AWS credential. The operator stores the AWS keys as
app secrets; the platform lands them in Secret Manager as
`<service>-aws-access-key-id` etc. and does **not** inject them as env vars, so
`catalog-api/src/secrets.rs` reads them at runtime (metadata-server access token
→ Secret Manager REST) and passes them to the `object_store` S3 builder. Locally
(and in tests) there is no metadata server: if `AWS_ACCESS_KEY_ID` is already in
the environment, or no service-name prefix is set, this is a no-op and the
env-based path (including a MinIO endpoint) is used unchanged. Non-secret S3
config (region, endpoint) travels as plain `[cloudrun].env_vars`.

## REST API surface

See [README.md](README.md#rest-api-surface) for the route list. The
authoritative source is `catalog-api/src/api.rs::api_router`.

- **Basic ops** (`/v1`): `ListNamespaces`, `DescribeNamespace`, `ListTables`,
  `DescribeTable`, `DeclareTable` (PUT — registers the table by writing its
  overlay + `owner`/`ttl_policy`, idempotent; the next sync fills its versions),
  `DeregisterTable` (DELETE — deletes the overlay, unregistering it; the next
  sync drops its derived entry). Namespaces are a derived view, not stored state.
- **`/ext` enriched surface**: `GET /ext/v1/tables` (full detail, no pagination),
  `GET /ext/v1/tables/:id/versions/:vid`, aux `sample`, version `protect`
  (writes the overlay), the TTL endpoints (`dryrun`, `apply`, `audit`),
  `GET /ext/v1/storage` (the storage-analysis breakdown), `GET /ext/v1/users`,
  and `GET /ext/v1/me` (caller identity + resolved role).
- Any instance serves any request; there is no leader-only 503.

## TTL engine

`catalog-core/src/ttl.rs` (eligibility) + `catalog-api/src/api.rs` (endpoints).
The TTL engine hard-deletes old timestamp version directories from S3 under a
per-table policy. Because the delete is irreversible, it is gated by a mandatory
dry-run-before-apply workflow, a `protected` flag, and a shape safety gate. Full
operational guidance is in [`docs/RUNBOOK-TTL.md`](docs/RUNBOOK-TTL.md).

### Eligibility (`ttl_eligible_versions`)

A version is eligible for deletion only if *all* of the following hold:

- it is not `protected` (never eligible, regardless of policy or shape);
- its shape passes the safety gate (`full`, `lance_only`, or `seg_only` —
  `lance_only_partial` and `empty` are refused);
- it fails *every* set threshold on the policy:
  - `keep_last_n` (if set): the version is not in the top-N most recent by
    `timestamp`.
  - `max_age_days` (if set): `now - timestamp` exceeds the threshold.
- If *neither* threshold is set (no policy), nothing is ever eligible. The
  AND-logic means a version is deleted only if it exceeds *both* thresholds when
  both are set (within either threshold => kept).

### Apply without a lock: the `deleting` marker flow

`ttl_apply` for a table closes the protect-vs-apply race with a conditional
overlay write instead of a lock:

1. **CAS #1** — recompute eligibility against the *fresh* overlay (policy +
   protected + shape gates) and, in the same conditional write, stamp a
   `deleting` marker on each eligible version. A concurrent `protect` that raced
   in since the read forces a Precondition retry that then excludes the
   newly-protected version.
2. **Delete** — resolve each eligible version's prefix (an unresolvable
   `snapshot_path` is refused, keeping the version rather than fake-succeeding)
   and hard-delete its entire `<table>/<timestamp>/` prefix tree.
3. **Audit** — append one `TtlAuditRecord` per deleted version to
   `_catalog/ttl_audit` (Lance `Append`), the durable evidence, *before* the
   markers are cleared.
4. **CAS #2** — a *succeeded* delete keeps its `deleting` marker as a tombstone:
   the version is gone from S3 but still in the derived snapshot until the next
   sync reconciles it out; the marker hides it from reads and keeps re-apply
   idempotent, and the sync clears the marker once the snapshot no longer lists
   the version. A *failed* delete's marker is cleared so a retry re-attempts it.
   `protected` is pruned for deleted versions.

Idempotent re-apply: the `deleting` markers exclude already-deleted versions from
the recomputed eligible set, so re-apply finds nothing. A mid-apply crash leaves
markers that a later apply or the sync reconciles. See
[`docs/RUNBOOK-TTL.md`](docs/RUNBOOK-TTL.md) for the audit caveats.

## Frontend

`frontend/` is a vanilla TS + Vite static SPA (hash-based routing, no framework).
The list view reads `/ext/v1/tables`; the drill-down reads `/v1/table/:id`; the
TTL view calls dry-run + apply (with a type-the-table-name confirmation gate
before the irreversible POST) and the audit endpoint. It calls the API
same-origin (default base `""`), so serving it from the same Cloud Run service
means no CORS. The container's frontend build stage produces `dist/`, which the
server serves at `/` (`CATALOG_WEBUI_DIR`).

## Observability

Structured JSON logs go to stdout for Cloud Logging: the sync summary
(tables checked, failed tables, duration), per-table sync failures, and TTL
outcomes. HTTP request/latency (RED) metrics come from Cloud Run's built-in
Cloud Monitoring; there is no Prometheus endpoint or scrape. Log-based metrics
and alerts can be layered on in Cloud Monitoring later.

## Notable behaviors and limitations

- **Snapshot last-wins can write an older sync over a newer one.** Bounded by
  the sync cadence and self-healing on the next sync; acceptable for
  recomputable derived state.
- **TTL audit-vs-registry is not one atomic write.** The audit `Append` precedes
  the marker/snapshot changes, so a mid-apply crash can produce a *duplicate*
  audit record on retry but never a *lost* one. See
  [`docs/RUNBOOK-TTL.md`](docs/RUNBOOK-TTL.md) caveat (b).
- **`reclaimable_bytes` is logical/deduped size, not physical footprint.** See
  [`docs/RUNBOOK-TTL.md`](docs/RUNBOOK-TTL.md) caveat (a).
- **App-level auth is delegated to the platform.** IAP + Trident authorize human
  requests (`allowed_usergroups`); the Cloud Scheduler service account reaches
  `/internal/jobs/sync` via its platform identity. The app does no auth of its
  own.
- **`/ext/v1/tables` has no pagination.** Fine at admin scale (dozens of tables).
- **Design features not implemented:** lineage, MCP, Flight SQL, profiling,
  maintenance/compaction, cost analysis, credential vending, operation history,
  inbox. See [`docs/design.md`](docs/design.md).
