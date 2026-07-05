# Architecture (as-built)

This document describes the system as implemented. The broader long-term vision
lives in [`docs/design.md`](docs/design.md).

## Overview

One binary, `catalog-api`, runs as a single stateless container on Cloud Run. It
serves the REST API and the SPA, and runs the S3 sweep and the TTL engine. There
is no leader election, no lock, and no database: **S3 is the sole store**. Any
instance can serve any request. The sweep runs request-scoped, triggered by
Cloud Scheduler — Cloud Run throttles CPU between requests, so a background loop
would freeze while idle.

```
Cloud Scheduler (cron) ─▶ POST /internal/jobs/sweep ─┐
                                                      │
Browser ─ IAP/Trident ─▶ catalog-api (Cloud Run, axum)
                          ├─ REST /v1 + /ext  (merged read model)
                          ├─ static SPA       (same origin, no CORS)
                          └─ sweep + TTL      ◀────────┘
                                    │
                                    ▼  S3 (sole store)
  <table>/<ts>/...            immutable version dirs; LIST/open; TTL deletes
  _catalog/registry          derived snapshot (Lance), sweep-written, last-wins
  _catalog/meta/<id>.json    authored overlay {owner, ttl_policy, protected[],
                             deleting[]} — ETag-CAS mutations
  _catalog/ttl_audit         append-only audit log (Lance)
```

## State model: derived vs authored

Each catalog version is one **immutable** timestamp directory
(`<table>/<ts>/dataset.lance/...`), never rewritten after export. That
immutability splits registry state in two:

1. **Derived state** — versions, shapes, byte splits, row counts, schemas, aux
   entries. A pure function of immutable S3 content, so any full sweep
   regenerates it. The derived snapshot (`_catalog/registry`, a Lance dataset)
   is therefore written **whole, last-wins, with no coordination**: two
   concurrent sweeps, or a Scheduler double-fire, cannot corrupt it — the worst
   case is bounded staleness until the next sweep. Lost-update protection is
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
swept. This is what lets the same full sweep root serve a scoped catalog — an
operator registers the handful of tables to track, and unregistered tables on S3
are never touched.

Everything the old k8s leader + write-lock protected was the *entanglement* of
these two in one read-modify-write blob. Separating them dissolves the
coordination problem instead of porting it. A read merges the two back into the
`TableEntry` wire shape the REST API and frontend already expect
(`catalog-api/src/catalog.rs`): the snapshot supplies derived fields, the
overlay supplies `owner`/`ttl_policy`/`protected`; a table registered but not yet
swept (declared before its first sweep) is materialized as a stub.

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
> dev) or `InMemory` (tests). The sweep/data path still uses `LocalFileSystem`
> freely — it does no conditional writes.

## The sweep

`catalog-store/src/sweep.rs` classifies one table's directory
(`sweep_table`). `catalog-api/src/sweep.rs` wraps it into one request-scoped
pass: `run_sweep` reads the registered set (the overlay objects), re-derives each
registered table's version set from S3, writes the whole snapshot as exactly that
set (last-wins), reconciles stale overlay markers, and best-effort prunes old
registry manifest versions. It never enumerates the root, so unregistered tables
cost nothing and never appear.

Because the snapshot is rebuilt from the registered set each pass, it stays an
allowlist automatically: a deregistered table (overlay deleted) is simply not
swept and drops out of the snapshot, and within a table, a version no longer on
S3 (removed out of band, or by a TTL apply) is gone from the freshly-derived set.
A registered table with no directory on S3 yet sweeps cleanly to zero versions (a
stub) until data lands.

A single registered table failing to sweep is isolated — logged, counted, and
kept from the prior snapshot — rather than aborting the whole cycle and stalling
every other table's freshness. A version transiently seen as `partial` (e.g. its
dataset momentarily failed to open) is re-derived cleanly by a later sweep, so it
does not stay permanently partial; a transient partial re-observation shows for
one cycle and self-heals, and the shape safety gate blocks TTL during that window
(the safe direction).

**Overlay reconciliation.** After writing the snapshot, the sweep prunes overlay
`protected`/`deleting` entries whose version id is no longer in the snapshot —
clearing the tombstone a TTL apply leaves behind once the version is truly gone
(see [TTL engine](#ttl-engine)). It never touches `owner`/`ttl_policy`.

**Idempotent under double-fire.** The `/internal/jobs/sweep` endpoint serializes
sweeps on a single instance with an in-process mutex (so a Scheduler double-fire
doesn't run two at once on the same instance); cross-instance concurrency is safe
regardless because the snapshot is last-wins.

### Version discovery

For each registered table the sweep lists that table's directory
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
  `dataset.lance/` (alongside lance-core directories). The sweep reads them from
  there.
- **Post-cutoff** (>= 2026-06-26): sidecar directories move to a top-level
  `dataset.sidecar/`; `dataset.lance/` is clean (lance-core only).
- **Transition** (~2026-06-12): sidecar content is duplicated both inside
  `dataset.lance/` and at top-level `dataset.sidecar/` (dual-write). The sweep
  deduplicates: when a top-level `dataset.sidecar/` is present it is preferred;
  otherwise sidecar bytes are read from inside the main lance dir.

The sweep supports both layouts so the full version history is cataloged
correctly. The real-world layout is captured in
[`docs/design.md`](docs/design.md), including concrete annotated example tables.

## The registry (derived snapshot)

The registry is a Lance dataset under `_catalog/registry` (path configurable via
`CATALOG_REGISTRY_PATH`). It stores one derived `TableEntry` per table: id, name,
namespace, root_location, last_swept, versions[], and aux_latest[] — plus
`owner`/`ttl_policy`/per-version `protected` slots that the sweep leaves empty
and the overlay fills in at read time. Each `TableVersion` carries version_id,
timestamp, snapshot_path, shape, partial, protected, the per-component byte
totals, row_count, num_fragments, schema_json, num_indices, aux[], and swept_at.

The persisted registry types are forward/backward compatible (absent fields
default), so a schema addition does not break an older reader or a snapshot
written by an older writer. The sweep tail-calls Lance `cleanup_old_versions` on
the registry so its manifest history does not grow unbounded (each sweep adds one
version).

Registry reads fail closed: `read_registry` returns `Ok(None)` only for a
genuinely absent dataset (first boot); any other read failure is an `Err`.

## The read model (merged view)

`catalog-api/src/catalog.rs` holds the merged read model, shared across handlers.
It reads the snapshot + all overlays and merges them into the `TableEntry` wire
shape. Because Cloud Run throttles CPU between requests, there is no background
refresh loop: the view is revalidated **lazily** — re-read from storage when
older than `CATALOG_CACHE_TTL_SECS` (default 5s), single-flighted so a burst of
stale reads triggers one reload — and immediately after a mutation invalidates it
(read-your-writes on the same instance). On a revalidation error the last good
view is kept and served; staleness is bounded by the sweep cadence anyway.
`/readyz` reports ready once the view has loaded at least once.

Mutations re-fetch the overlay fresh inside every write (the cache is for reads
only): a handler calls `MetaStore::mutate_meta`, then invalidates the read cache.

## Startup and shutdown

At startup the process fetches AWS credentials from Secret Manager (see
[Credentials](#credentials)), builds the sweep + overlay object stores, and binds
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
  overlay + `owner`/`ttl_policy`, idempotent; the next sweep fills its versions),
  `DeregisterTable` (DELETE — deletes the overlay, unregistering it; the next
  sweep drops its derived entry). Namespaces are a derived view, not stored state.
- **`/ext` enriched surface**: `GET /ext/v1/tables` (full detail, no pagination),
  `GET /ext/v1/tables/:id/versions/:vid`, the TTL endpoints (`dryrun`, `apply`,
  `audit`), and version `protect` (writes the overlay).
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
   sweep reconciles it out; the marker hides it from reads and keeps re-apply
   idempotent, and the sweep clears the marker once the snapshot no longer lists
   the version. A *failed* delete's marker is cleared so a retry re-attempts it.
   `protected` is pruned for deleted versions.

Idempotent re-apply: the `deleting` markers exclude already-deleted versions from
the recomputed eligible set, so re-apply finds nothing. A mid-apply crash leaves
markers that a later apply or the sweep reconciles. See
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

Structured JSON logs go to stdout for Cloud Logging: the sweep summary
(tables checked, failed tables, duration), per-table sweep failures, and TTL
outcomes. HTTP request/latency (RED) metrics come from Cloud Run's built-in
Cloud Monitoring; there is no Prometheus endpoint or scrape. Log-based metrics
and alerts can be layered on in Cloud Monitoring later.

## Notable behaviors and limitations

- **Snapshot last-wins can write an older sweep over a newer one.** Bounded by
  the sweep cadence and self-healing on the next sweep; acceptable for
  recomputable derived state.
- **TTL audit-vs-registry is not one atomic write.** The audit `Append` precedes
  the marker/snapshot changes, so a mid-apply crash can produce a *duplicate*
  audit record on retry but never a *lost* one. See
  [`docs/RUNBOOK-TTL.md`](docs/RUNBOOK-TTL.md) caveat (b).
- **`reclaimable_bytes` is logical/deduped size, not physical footprint.** See
  [`docs/RUNBOOK-TTL.md`](docs/RUNBOOK-TTL.md) caveat (a).
- **App-level auth is delegated to the platform.** IAP + Trident authorize human
  requests (`allowed_usergroups`); the Cloud Scheduler service account reaches
  `/internal/jobs/sweep` via its platform identity. The app does no auth of its
  own.
- **`/ext/v1/tables` has no pagination.** Fine at admin scale (dozens of tables).
- **Design features not implemented:** lineage, MCP, Flight SQL, profiling,
  maintenance/compaction, cost analysis, credential vending, operation history,
  inbox. See [`docs/design.md`](docs/design.md).
