# Lance Data Catalog Service — Design Document (v3, consolidated)

A custom data catalog service, implemented in Rust, that reports on and manages Lance tables — and their auxiliary sidecar tables — stored on S3. It provides table discovery, data inspection, manifest inspection, data profiling, lineage tracking, version management, retention/TTL control, cost analysis, self-monitoring, and an MCP interface for agents. It conforms to the Lance Namespace REST spec at the basic-operations tier and extends it with governance/reporting capabilities. It runs on Kubernetes, targets 10k QPS, and manages 10k+ tables.

This supersedes the earlier v1 recap, v2 design, the conformance review, and the monitoring component — all corrections and additions are folded in here.

**Design pillars — every decision serves one or more:**
- **Reliable** — S3 as the only source of truth; at-least-once delivery with idempotent apply; degraded mode equals normal mode; split-brain degrades to a commit conflict, never corruption.
- **Scalable** — lock-free in-memory reads; load bounded by distinct dirty tables not event volume; QoS isolation of heavy work; horizontal read scaling with zero coordination.
- **Maintainable** — one manifest set from laptop to prod; spec-pinned generated models; the whole system is one idempotent apply function fed by three convergent loops (the Kubernetes controller pattern).

---

## 1. Core Architecture Principles

1. **S3 is the sole source of truth.** No Postgres, no Redis, no broker. Lance manifests are authoritative for table state — Lance commits bypass any catalog (S3 conditional PUTs give commit atomicity natively), so a relational registry would only be authoritative for naming, and even that is unnecessary here.
2. **The catalog's own state is Lance tables** under a `_catalog/` prefix — registry, lineage, policies, profiles, cost snapshots, checkpoints, job state. Durability from S3, queryability via DataFusion, and dogfooding the format. (Parquet sidecars are *tracked*, but catalog-internal state is always Lance.)
3. **Conform to the Lance Namespace REST spec** for the core API so namespace client SDKs (Spark, Ray, Trino, pylance) connect for free; everything non-spec lives behind an `/ext` prefix. Pin the OpenAPI version — the upstream Namespace/Catalog split is still in flight.
4. **Stay out of the commit path.** The service is a registry + observer + control plane. Basic ops are metadata-only (DeclareTable / DeregisterTable / DescribeTable-returns-location); data ops are delegated to the Lance SDK on the client side.
5. **No multi-table transactions** (explicit scoping decision — removes the main justification for a relational DB).
6. **Single-writer control plane, idempotent apply, three convergent loops** — in-memory push (fast, unreliable) → durable inbox tail (at-least-once) → manifest sweep (slow, ground-truth), all funneling into one idempotent apply. This is the controller/reconciler shape.

---

## 2. Storage Layout

```
s3://…/_catalog/
  registry/                # namespace hierarchy + table→location + aux attachments (single root manifest → CAS mutations)
  table_snapshots/         # cached per-table: latest version, schema, fragment/index/deletion stats; aux fingerprints
  table_profiles/          # data-distribution profiles per (table, version, column)
  table_operations/        # durable per-version operation log (survives Lance transaction-file pruning)
  table_indices/           # current index set + per-table index lifecycle deltas
  lineage_events/          # raw OpenLineage event log (append-only)
  policies/                # retention/maintenance policies
  cost_snapshots/          # daily per-table referenced-vs-actual bytes
  jobs/                    # maintenance job state, schedules, checkpoints, audit, dead-letters
  inbox/                   # durable ingest queue: {ts}-{pod}-{uuid}.json, 24h lifecycle expiry
```

- The full hierarchy lives in **one root registry manifest**, so rename/move-across-namespace is a single CAS mutation (no multi-key atomicity problem).
- `_catalog/` tables get their own compaction/cleanup — the control plane maintains itself (lineage and inbox-derived tables are append-heavy).
- The `_catalog/` prefix is **excluded from ListTables** so system tables never surface as user tables (they remain queryable as an admin capability).

---

## 3. Table Model: Main + Auxiliary Tables

Each catalog entry is a **table group**: one main Lance table plus zero or more auxiliary sidecar tables (Lance or Parquet) — per-table stats extracts, embeddings, frame indexes, etc. Lance has no native table-grouping concept, so the catalog owns this model.

### 3.1 Registry model

```jsonc
{
  "id": "tbl_9f2c…",                 // stable catalog id; survives renames
  "name": "scenario_exports",
  "namespace": ["av", "perception"],
  "location": "s3://bucket/av/scenario_exports.lance",
  "format": "lance",
  "aux": [
    { "name": "column_stats",  "role": "stats",      "format": "parquet",
      "location": "s3://bucket/av/scenario_exports_stats/", "lifecycle": "coupled" },
    { "name": "clip_embeddings","role": "embeddings", "format": "lance",
      "location": "s3://bucket/av/scenario_exports_emb.lance", "lifecycle": "coupled" }
  ]
}
```
`role` ∈ stats | embeddings | index | derived | custom:\*. `lifecycle` ∈ coupled (dropped/retained with parent) | independent.

### 3.2 Discovery & registration (precedence: explicit > event > convention)

1. **Explicit** — `PUT /ext/v1/tables/{id}/aux/{name}` attaches an existing location.
2. **OpenLineage events** — a Spark job writing main + sidecars emits all outputs in one run; the ingest pipeline groups outputs by run and attaches sidecars using role hints from job facets or naming.
3. **Convention scan** — the sweep recognizes configured suffix patterns (`_stats`, `_emb`, `{table}_aux/{name}/`) and *proposes* attachments (flagged `discovered: true`, promotable via API) — proposal-only to avoid false positives.

### 3.3 Sync semantics per format

- **Lance aux** — identical machinery to main tables: version-number CAS, N+1 manifest probe in sweep, full stats snapshot.
- **Parquet aux** — no version protocol, so define a **fingerprint** = hash over the sorted set of (key, ETag, size) from a LIST of the prefix; idempotent apply becomes fingerprint compare-and-refresh. Sweep probe = one LIST per parquet aux (cheap; aux prefixes are small). Prefer a single `_SUCCESS`/manifest object over listing when writers provide one.
- **Version correlation** — when an OL run groups main + aux outputs, record `parent_version_at_sync` on the aux snapshot. Expose `aux_staleness` (parent versions elapsed since aux last synced) — the "embeddings are 12 versions behind the table" signal no generic catalog provides.

### 3.4 Cascade behavior

- Deregister/drop of parent → `coupled` aux deregistered/dropped too; `independent` aux detached and re-homed as standalone entries.
- Health, cost, and TTL views roll aux into the parent by default with per-aux drill-down.
- In the lineage graph, aux tables are first-class nodes with an `attached-to` edge to the parent plus their own produced-by/consumed-by edges.

---

## 4. Lance Namespace Spec Conformance

Verified against the live spec (operation set, `DescribeTableResponse`, `ListTablesResponse`). Re-verify against the pinned OpenAPI (`docs/src/spec.yaml`) at build time. The design implements the recommended basic operation set and layers governance/reporting on top.

**Conformance strategy in one line:** implement the spec operations against Lance-native truth, carry namespace-managed metadata (including aux pointers) in `properties`, keep everything non-spec on an `/ext` prefix, and return `managedVersioning=false` so Lance stays the version authority.

### 4.1 Cross-cutting contracts (routing/model layer)

- **Basic op set is exactly** CreateNamespace, ListNamespaces, DescribeNamespace, DropNamespace, DeclareTable, ListTables, DescribeTable, DeregisterTable. DescribeTable basic behavior returns the table `location` only, without opening the dataset (matches the in-memory serving model). CreateTable/DropTable are not catalog primitives — fulfilled via Declare/Deregister + Lance SDK.
- **`id` optional in body, carried in the route path** (`/v1/table/{id}/describe`); the server must **400 if path id ≠ body id**.
- **Numeric ErrorResponse** adopted verbatim (1=NamespaceNotFound, 3=NamespaceNotEmpty, 4=TableNotFound, 5=TableAlreadyExists, 11=TableVersionNotFound, 15=PermissionDenied, …) — no invented error strings.
- **Operation versioning convention** `<OpId>V<n>` applied to any custom/extension operations too.

### 4.2 `ListTables` is names-only — enrichment is an extension

The spec `ListTablesResponse` carries exactly a set of full identifier strings plus an optional `pageToken` — not even a version number. Therefore:
- Standard `ListTables` returns identifier strings + page token, unchanged.
- The enriched listing (versions, aux summary, health, freshness) is a **catalog extension**: `GET /ext/v1/tables?expand=aux,stats,health`, returning a catalog-defined model. Spec clients are unaffected; dashboards/agents use the extension.

### 4.3 Aux metadata rides `properties` (the semantically correct field)

`DescribeTableResponse.properties` (`Map<String,String>`) is defined as *information managed by the namespace* — distinct from `metadata`, which is table-intrinsic and requires opening the dataset. Aux pointers are namespace-managed, so `properties` is correct, not a workaround. Reserved-key scheme (`catalog.*` namespace):
```
catalog.aux.count = "2"
catalog.aux.0.{name,role,format,location,lifecycle,parent_version_at_sync} = …
```
Pure spec clients ignore unknown keys; the typed `aux[]` structure is served by the `/ext` endpoints. **Aux tables are not registered as first-class namespace tables** (they'd pollute ListTables, which every engine sees); they live as parent properties, and are *promoted* to real registered tables (via RegisterTable) only when `lifecycle=independent` and a consumer needs them addressable.

### 4.4 Adopt native operations instead of custom ones

The spec surface is much larger than the basic set; several features map onto native ops so spec clients get them for free (internal richness exposed as an `/ext` superset):

| Capability | Native op(s) to serve through | Note |
|---|---|---|
| Table stats | `GetTableStats` + `TableBasicStats`/`FragmentStats`/`FragmentSummary`; also `DescribeTable.stats` | snapshot maps onto these |
| Index staleness | `DescribeTableIndexStats`, `ListTableIndices` | `num_unindexed_rows` is native |
| Version restore | `RestoreTable` | not a custom endpoint |
| Rename/move | `RenameTable` | implemented by the single-root-manifest CAS |
| Versions & tags | `ListTableVersions`, `DescribeTableVersion`, tag CRUD ops | backed by Lance-native history |
| Query/sample | `QueryTable`, `CountTableRows`, `ExplainTableQueryPlan` (Arrow IPC; vector + FTS) | Flight SQL is the ADBC value-add |
| Governed storage access | `storageOptions` + credential vending (`vend_credentials`, `expires_at_millis`) | catalog mints scoped temp creds — key for the MCP/agent path |
| Schema edits (if exposed) | `UpdateTableSchemaMetadata`, `AlterTable{Add,Alter,Drop,Backfill}Columns` | native, not custom |

**`managedVersioning=false`** is returned because Spark commits directly to Lance: clients use Lance-native versioning; namespace version *read* ops are served from Lance history, and version *management* ops are not the authority. **`isOnlyDeclared`** is honored for name-reserved-but-no-data tables.

---

## 5. Feature Design

### 5.1 Inspection & table stats — Lance's native Rust APIs (no Python)

The `lance` crate exposes everything directly: the **dataset statistics module** (fragment/small-file/deleted-row counts), **index statistics** (`num_indexed_rows`/`num_unindexed_rows` per index), **fragment/data-file APIs** (per-fragment files, sizes, deletion files), **versions/tags/transactions**, table config key-values (read + write), and **incremental IO-stat counters** (used to meter the catalog's own S3 traffic per loop). The refresher materializes all of this into `table_snapshots`; the API serves from memory and maps onto native `GetTableStats`/`DescribeTableIndexStats` (§4.4).

### 5.2 Data profiling / pattern distribution — DataFusion job (no native API)

Lance has no native column-distribution API (v2.1 page stats are pushdown internals), so profiling is a first-class catalog job:
- Engine: embedded **DataFusion via `lance-datafusion`**. Per column class: min/max/null_count/`approx_distinct` for all; `approx_percentile_cont` histograms for numerics; top-K for low-cardinality strings; length stats for blobs; dimension + norm stats on a sample for vectors.
- Sampling: exploit Lance's random-access strength — profile N rows (default 100k) for large tables; record `sample_fraction`.
- Storage: `table_profiles` keyed by `(table_id, version, column)` → versioned, diffable ("distribution drift between v400 and v412"), queryable via Flight SQL.
- Trigger: on-demand API or post-sync policy (every Nth version / on schema change); never inline with a read. Runs in the inspection QoS pool (§11), never the metadata runtime.

### 5.3 Manifest inspection

Virtual metadata tables per table — `$versions`, `$fragments`, `$files`, `$indices`, `$transactions`, `$operations` — materialized on demand from the lance crate and queryable via Flight SQL (Iceberg metadata-tables pattern). Cross-table queries ("all fragments <10MB anywhere") come free. `$transactions`/`$indices` reflect current on-disk state; `$operations` is the **durable** enriched history from §5.10 that survives Lance's transaction-file pruning.

### 5.4 Data query

Native `QueryTable`/`CountTableRows`/`ExplainTableQueryPlan` for spec conformance (Arrow IPC, vector + FTS), plus an **Arrow Flight SQL** endpoint (tonic) as the ADBC/JDBC value-add. Read-only enforcement in the SQL planner (reject non-SELECT), row/byte/time caps, per-query memory limit.

### 5.5 Lineage

- OpenLineage HTTP ingest endpoint; Spark emits via the OL listener in **async mode** (job-commit latency never couples to the catalog).
- Raw events → `lineage_events`; projected dataset-level edges → in-memory petgraph behind `ArcSwap`, checkpointed periodically, rebuilt on start from checkpoint + tail replay.
- Normalize OL dataset naming (`s3a://` vs `s3://`, trailing slashes) to catalog identity at ingest or the graph fragments; dedup on `(runId, eventType)`.
- **Lineage is the only stream the sweep cannot recover** (manifests don't record reads) → it alone justifies the durable inbox (§7).

### 5.6 Version management

Served via native `RestoreTable`/`RenameTable` and native tag/version ops backed by Lance history: version list with operation type + row deltas (from transaction metadata), tag CRUD (pin `prod`), schema diff between versions, and profile-drift diff (§5.2).

### 5.7 Retention / TTL — native knobs first, orchestrate the rest

- Lance table config supports **`lance.auto_cleanup.interval`** and **`lance.auto_cleanup.older_than`** (cleanup runs during optimize). For a simple age policy the TTL engine's first action is a **config write on the table**, not a scheduled job. The scheduler remains for `keep_last_n`, tag-protection, and lineage-aware rules.
- **Lineage-aware retention** (differentiator): never reap a version referenced as a job input within N days — a join of `lineage_events` × version metadata. Reproducibility protection no generic catalog offers.
- Surface, via a dry-run endpoint, "reclaimable bytes" and "rollback horizon" before apply (compaction alone frees nothing; prune permanently removes rollback for pruned versions).
- Parquet aux (no versions) → retention via catalog-managed S3 lifecycle rules; `coupled` aux inherit the parent policy.

### 5.8 Maintenance engine — conflict-aware by design

Catalog-run compaction races Spark writers under Lance's optimistic concurrency, and compaction classically conflicts with index building, degrading layout on repeated retries. Encode best practices:
- **Compact with `defer_index_remap` (Fragment Reuse Index)** so compaction no longer conflicts with concurrent index builds/appends — essential under continuous Spark ingest; schedule a periodic remap to burn down FRI debt.
- Trigger defaults from upstream guidance: keep fragments ~O(100) until ~1B rows; optimize after ≥100k modified rows or ≥20 modification ops — driven by snapshot stats already held.
- Bounded retry with backoff + jitter on commit conflicts; a per-table maintenance mutex so the catalog never races itself.
- Post-compaction, trigger incremental index optimize so `num_unindexed_rows` returns to ~0.
- Prioritize the queue by reclaimable bytes × query heat (cost snapshots × read-repair counters) — cost-aware scheduling from tables already held.

### 5.9 Cost analysis

Per table (aux rolled up): *referenced bytes* (fragment sizes from current manifest — cheap) vs *actual prefix bytes* (S3 Inventory daily; ListObjects on demand). Delta = reclaimable-by-cleanup → "policy X frees $Y/mo," feeding maintenance prioritization. Snapshot daily into `cost_snapshots`.

### 5.10 Table activity — indices, transactions & operation history

The catalog reflects the **indices, transactions, and operations** that occurred on each Lance table. The design splits cleanly by what Lance exposes officially (use directly) versus what must be built — and the built piece is modularized for upstream contribution.

**What Lance provides natively (use directly):**
- *Indices* — `Dataset::list_indices()` returns per-index `{name, type, columns, uuid, covered fragment_ids, built-at dataset version}`; `index_stats(name)` returns `{num_indexed_rows, num_unindexed_rows, index_type, distance_type, num_indices}`; IndexMetadata lives in the manifest. Namespace-level: `ListTableIndices`, `DescribeTableIndexStats`.
- *Transactions* — every version's transaction is readable via `Dataset::read_transaction()` (current) and `get_transaction(version)`, returning `{read_version, uuid, operation}`, where `operation` is Lance's typed enum (Append, Delete, Overwrite, Update, Merge, Rewrite/compaction, CreateIndex, Project, UpdateConfig, DataReplacement, Clone, …). Writers may attach `transaction_properties` and a commit message.
- *Versions* — `versions()`/`list_versions()` returns each version's number + timestamp.

**The gap — no official operation-history API.** `versions()` yields number + timestamp but **not the operation**; recovering what happened at each version means reading that version's transaction one at a time, and there is no `Dataset::history()`/changelog returning the normalized `(version, op_type, summary, read_version, uuid, ts)` sequence. Worse, **transaction files are pruned by cleanup** (7-day default; tagged versions exempt), so `get_transaction(old_version)` returns None after retention — historical operation detail disappears from Lance once versions age out.

**Design consequence — capture at observe-time, persist durably.** Because Lance doesn't retain operation history, the catalog records it when it first observes a version (transaction file still fresh) and stores it permanently. The catalog thereby becomes the **durable operation-history authority that outlives Lance's transaction retention** — additive, not redundant.

Three sub-components:
1. **Index reflector** (native-tool-backed) — each sync snapshots the index set (`list_indices` + `index_stats`) into `table_indices`, diffs against the prior snapshot to derive lifecycle events (created = new uuid; rebuilt = uuid/version changed; dropped = uuid gone; staleness = unindexed rows), and folds those events into the operation log.
2. **Operation reflector** (native-tool-backed) — on observing new version N, reads transaction(N), classifies the op, extracts a compact typed summary (fragments ±, rows added/deleted where derivable, delete predicate, projected/merged columns, CreateIndex target, …), captures `read_version`/`uuid`/`transaction_properties`/commit message, and appends one idempotent record keyed by `(table_id, version)` into `table_operations`. Distinguishes **system** ops (compaction/index/cleanup — including the catalog's own maintenance) from **data** ops (Spark writes).
3. **History assembler** — serves the changelog from `table_operations` (durable, survives pruning) as the primary path; for a table first seen mid-life, best-effort backfills by iterating `versions()` + `get_transaction(v)` over still-present transaction files, marking records `backfilled` and the range `partial` where transaction files were already pruned.

**Modularized for OSS contribution.** Sub-component 3 — iterating versions and normalizing their transactions into a typed operation log — is generally useful and missing from Lance, so it is built as a dependency-light Rust module (`lance-history`) depending only on `lance`/`lance-table`/`arrow`/`futures`, with no catalog/S3/DataFusion coupling:
- **API**: `history(&Dataset) -> Stream<OperationRecord>` over the available version range, plus `operation_at(&Dataset, version)`; **reuse Lance's own `Operation` enum** rather than redefining it (the contribution is the summarization + history iteration over the existing type), and gracefully surface the pruned-transaction `partial` case.
- **Output** also as Arrow RecordBatches so it plugs into the catalog's Flight SQL / metadata-tables surface as the `$operations` virtual table (Arrow-native output matches Lance conventions).
- **Path**: vendor it as a submodule now (it works today against the public `read_transaction`/`get_transaction`/`versions` APIs) and upstream it as `Dataset::history()` / an operations iterator; drop the vendored copy once merged. A read-only, format-native addition filling an obvious gap — a strong contribution candidate.

**Integration:** the reflectors run inside the existing idempotent apply (op records and index snapshots are keyed, dedup-safe); `table_operations`/`table_indices` are new `_catalog` tables; `$operations`/`$indices`/`$transactions` are queryable metadata tables (§5.3); the API adds `list_operations`, `describe_operation`, `list_index_events`, and native version-list responses (`ListTableVersions`/`DescribeTableVersion`) are enriched with operation type via `/ext`. Recommend Spark writers set `transaction_properties` (job id, pipeline, actor) so provenance is captured natively — and the operation log is what makes lineage-aware TTL, audit, and "what happened between v400 and v412" answerable even after Lance prunes those versions.

---

## 6. MCP Service for Agents

Exposes MCP so agents (Claude Code, internal agents) query and act on the lakehouse through governed tools rather than raw S3.

- **SDK**: official **`rmcp`** with `transport-streamable-http-server`; `StreamableHttpService` nests into the existing axum router (`.nest_service("/mcp", …)`) — same process, same `ArcSwap` state, same tower middleware. `#[tool_router]`/`#[tool]` macros define tools; `schemars` derives schemas from the same types the REST API uses.
- **Positioning**: existing LanceDB MCP servers operate at row/vector-search level over one database; this is the **metadata/governance layer** (discover, inspect, lineage, health, cost) — complementary and a differentiator.
- **Read tools (default)**: `search_tables`, `describe_table`, `get_table_profile`, `sample_rows`, `query_table` (read-only, capped), `get_lineage`, `list_versions`/`diff_versions` (each step annotated with the operation that caused it), `get_table_history` / `list_index_events` (§5.10), `get_cost_summary`, `get_health_report`.
- **Write tools (opt-in scope, audited)**: `tag_version`, `set_policy`, `request_maintenance` — never destructive; drops/restores stay human-API-only.
- **Guardrails**: read-only by default; every result carries an `as_of` freshness stamp; hard pagination/result caps (agent retry loops amplify load); MCP traffic in the inspection QoS tier (§11) so agent scans can't starve metadata p99; OAuth bearer auth + per-tool authorization; DNS-rebinding protection (rmcp); **governed data access via native credential vending** (§4.4) so agents get scoped, expiring S3 creds; full audit of tool calls into `_catalog/jobs`.
- **Agent ergonomics**: stable `table_id`s in every response; embedded next-step hints; an MCP *resource* exposing a compact per-table "card" for cheap grounding before tool calls.

---

## 7. Sync / Update Ingestion (Spark → Catalog)

Settled requirements: **at-least-once delivery, idempotent apply, S3 as the only stateful backbone.** No Kafka/SQS/EventBridge/broker (volume is trivial — worst case ~1.7k events/sec of KB JSON — and refresh is level-triggered, needing no ordered durable log).

- **Fast-ack edge**: any API pod accepts `POST /api/v1/lineage`; validate → buffer ~1s → one batched PUT to `inbox/{ts}-{pod}-{uuid}.json` → ack. Also fire-and-forget forward in-memory to the leader (fast path).
- **Consumption**: leader tails `inbox/` (LIST after high-water marker, ~1s), processes, advances checkpoint. No delete-behind — 24h lifecycle expiry doubles as a replay buffer (reset marker to recover from bugs). Clock skew absorbed by re-listing marker−60s + processed-keys LRU (over-read is harmless).
- **Two consumers, two semantics**: lineage appender wants every event (batch into 1 Lance commit/sec); snapshot refresher wants latest-per-table — a coalescing dirty-set (`DashMap<TableId, max_seen>`) + semaphore-bounded worker pool (~128 concurrent manifest reads). Queue depth bounds by *distinct dirty tables*, not event volume.
- **Idempotency keys from domain identity, never delivery artifacts**: Lance version number (from storage) for main + Lance aux; prefix fingerprint for Parquet aux; `(runId, eventType)` for lineage. Max-merge CAS; never regress a snapshot.
- **Aux in events**: OL runs listing multiple outputs attach/refresh aux in the same apply; record `parent_version_at_sync`.
- **Poison events**: after N failed applies → `inbox/deadletter/` with error context + alert; never block the tail.

Push vs pull is layered on purpose: the in-memory push gives ~0 latency on the common path; the S3 inbox tail is the durable backbone that survives leader restarts. Duplicates across both channels are free (idempotent apply).

## 8. Reconciliation Sweep (Backstop)

- **Continuous trickle** (token bucket ~6 checks/sec ⇒ 10k tables / ~30 min), never periodic bursts. The rate knob *is* the staleness-SLO knob.
- **Cheap probe**: table known at version N → HEAD the N+1 manifest; miss = unchanged. (With descending zero-padded manifest naming, `LIST max-keys=1` returns latest directly — select probe per naming scheme.) Parquet aux: LIST-fingerprint compare.
- **Read-repair**: a DescribeTable hitting a stale snapshot enqueues an immediate refresh — hot tables self-heal faster than sweep cadence.
- Covers what the inbox can't: events never produced (Spark died pre-COMPLETE) and out-of-band writers. It cannot recover lineage — the asymmetry that motivates the durable inbox.

---

## 9. Caching, State & Bootstrap

- **In-process memory only; no Redis.** State for 10k tables ≈ 100–300MB. `ArcSwap<CatalogState>` snapshots for lock-free reads; petgraph behind the same pattern. A separate cache tier would add a third coherence domain and a network hop to serve data that fits in heap ×100.
- **S3 is the replication bus**: the leader checkpoints snapshots/changelog into `_catalog/` Lance tables; read replicas tail new versions every few seconds and hot-swap. Lance versioning *is* the changelog — no pub/sub, no gRPC mesh.
- **Bootstrap = checkpoint the derived cache itself**: registry manifest (~10MB) → policies → `table_snapshots` checkpoint (one scan, not 10k dataset opens) → lineage checkpoint + tail replay. Readiness gates on the registry; everything else is stale-but-servable and reconciles in background. Cold start in seconds; deploys are cheap.
- Second cache class (manifest materializations, Flight SQL results, profiles): bounded `moka` LRU by bytes, spill to local NVMe (emptyDir) — never over the network.

---

## 10. Backend Stack

**Verdict: axum + tonic + rmcp on one tower stack.** actix-web benchmarks ~10–15% higher throughput, but at 10k QPS of small in-memory JSON both frameworks are >50× past requirement — the real bottlenecks are S3 latency, DataFusion CPU, and serde. The deciding factors are ecosystem, not req/s:
1. Arrow Flight SQL is tonic-based → tonic + axum share hyper/tower, giving one middleware stack (auth, limits, tracing, load-shed) across REST + gRPC.
2. rmcp nests natively into axum → MCP rides the same router/middleware.
3. Tower `Service` composition is exactly where QoS isolation (per-class concurrency limits + load-shedding) is implemented.
4. utoipa for OpenAPI aligned with the Namespace models; the tower idiom is the 2026 maintainability default.

Performance discipline that matters at scale: pre-serialize hot DescribeTable/ListTables payloads to bytes inside the snapshot (skip per-request serde); a separate tokio runtime (or dedicated pods) for DataFusion; `tower::limit` + `load_shed` on the inspection class.

Instrumentation crates (see §12): `metrics`, `metrics-exporter-prometheus`, `axum-prometheus`, `tokio-metrics`, `metrics-process`.

---

## 11. Kubernetes Deployment — One Manifest Set, Prod to Laptop

### 11.1 Topology (prod)

- **`catalog-api`** (stateless read tier): namespace ops, describe/list, lineage queries, MCP read tools. Hydrates from checkpoints, tails `_catalog/` versions. HPA on RPS (KEDA/Prometheus adapter — CPU is a poor proxy for memory-read work). ~3–6 pods, ~1 CPU / 2Gi. Mutations forwarded to the leader.
- **`catalog-controller`** (replicas=2, leader-elected via **k8s Lease API**, kube-rs): writer actors, inbox tail, schedulers, sweep, maintenance, self-compaction. Standby hydrates continuously (warm failover ~15s). **Lance conditional-put commits are the fencing backstop** — brief dual-leadership degrades to a commit conflict, never corruption.
- **`catalog-inspect`** (optional third deployment; day-one if the load test §16 says so): Flight SQL + profiling + heavy MCP tools; heavier pods, NVMe spill, own HPA. Minimum bar if colocated: separate tokio runtime + tower concurrency limits/load-shed so metadata p99 stays flat under scans.
- Mechanics: readiness gated on hydration staleness (checkpoint age), generous startup probe, preStop flush + final checkpoint on the controller, PDB + zone anti-affinity on the API tier, IRSA for S3, `maxSurge=1` rolling deploys.

### 11.2 Local development — k8s-native, no docker-compose

**One source of truth for manifests.** Kustomize `base/` + overlays; local runs the *same* base as prod.

```
deploy/
  base/
    <workloads, Services, RBAC, config>
    monitoring/                     # ServiceMonitor, PrometheusRule, dashboard ConfigMaps (§12)
  overlays/
    local/     # kind/k3d: 1 replica, in-cluster MinIO, low resources, debug logging
    staging/
    prod/      # IRSA, HPA, PDB, anti-affinity
Tiltfile
```

- **Cluster**: kind (or k3d) locally — including on the headless Ubuntu box; Tilt's web UI port-forwards over SSH; `allow_k8s_contexts` scopes Tilt to the kind context.
- **Dev loop**: Tilt with `k8s_yaml(kustomize('./deploy/overlays/local'))`. For Rust speed, use the compiled-binary live-update pattern (Cluster API style): `local_resource` runs `cargo build` (cross-compile to `x86_64-unknown-linux-musl` where host ≠ linux), then `custom_build` + `live_update=[sync(binary), restart]` — seconds per iteration. Fall back to `docker_build` with cargo-chef layer caching for full builds.
- **Local S3 = MinIO in-cluster.** **Critical parity check**: Lance commits depend on `If-None-Match: *` conditional PUT. MinIO has shipped conditional writes since 2023 but historically diverged from S3 on the `*` wildcard (expected exact ETags). **Pin a MinIO version verified against S3 semantics and add a CI conformance smoke test** (concurrent-commit: exactly one writer wins, loser gets 412). Fallback: LocalStack.
- **Local Spark**: a `spark-submit` Job in the local overlay with the OL listener pointed at the in-cluster catalog Service — the full write→event→sync loop runs on a laptop.
- **CI**: the same kind + local overlay boots in CI for integration tests — local, CI, and prod converge on one manifest lineage.

---

## 12. Self-Monitoring & Observability

### 12.1 Stance

- **Lean, pull-based, no collector.** The service exposes `/metrics`; Prometheus scrapes directly — no push gateway, no OTel collector, no sidecar on the metrics path. The only new parts are the standard cluster stack (Prometheus + Grafana + Alertmanager), usually already present.
- **k8s-native provisioning.** Scrape config, alerts, and dashboards are Kubernetes objects (ServiceMonitor, PrometheusRule, dashboard ConfigMaps) in the kustomize base, flowing through the same overlays. Grafana's sidecar auto-loads ConfigMaps labeled `grafana_dashboard: "1"` — dashboards are code, not clicks (also the correct pattern, since Grafana replicas don't persist UI-built dashboards across respawns).
- **No circular dependency.** Metrics land in Prometheus (separate) and state is S3, so monitoring never depends on the monitored service; the leader alert fires even if the control plane is wedged.
- **Cardinality is a hard constraint.** `table_id` is **never** a metric label (10k tables → millions of series). Operational metrics carry only bounded labels; per-table facts are API/`_catalog` queries; the aggregate distribution is captured with histograms.

### 12.2 Instrumentation stack (Rust)

Five pure-Rust additions, no FFI, no new services: `metrics` (facade), `metrics-exporter-prometheus` (global recorder + scrape render), `axum-prometheus` (auto HTTP RED on the same backend), `tokio-metrics` (runtime saturation), `metrics-process` (RSS/CPU/fds). One recorder; `GET /metrics` on an **internal** port only. Both deployments expose it; leader-only series read zero elsewhere and are disambiguated by `catalog_is_leader`. Flight SQL and MCP ride the same tower layer, so their RED metrics come from one middleware with a `class`/`protocol` label. Logs (structured JSON to stdout via `tracing`) and traces (OTLP→Tempo) stay separate and **optional**.

### 12.3 Metrics catalog (prefix `catalog_`; ★ = SLI)

**Request layer (RED)**: `http_requests_total`★ (route,method,status,class), `http_request_duration_seconds`★ (route,method,class), `http_requests_in_flight` (class), `http_response_bytes` (route,class), `load_shed_total`★ (class,reason), `flightsql_queries_total` (status), `flightsql_query_duration_seconds` (kind), `flightsql_rows_scanned_total`, `mcp_tool_calls_total`★ (tool,status), `mcp_tool_duration_seconds` (tool), `mcp_active_sessions`.

**Ingestion**: `ingest_events_received_total` (source,kind), `ingest_inbox_puts_total` (result), `ingest_inbox_batch_size`, `ingest_apply_total` (type,result), `ingest_apply_duration_seconds` (type), `ingest_dedup_dropped_total` (type), `ingest_deadletter_total`★ (type,reason), `ingest_inbox_lag_seconds`★ (age of oldest unprocessed object).

**Reconciler**: `dirty_set_depth`★, `refresh_workers_active`, `refresh_total` (trigger,result), `refresh_duration_seconds` (format), `refresh_coalesce_ratio`.

**Sweep**: `sweep_tables_checked_total` (probe_result), `sweep_cycle_duration_seconds`, `sweep_position_age_seconds`★, `sweep_drift_detected_total` (type).

**Freshness/convergence**: `snapshot_staleness_seconds`★ (histogram across all tables), `aux_staleness_versions` (role), `hydration_ready`★, `hydration_age_seconds`.

**Lineage**: `lineage_events_appended_total`, `lineage_graph_nodes`/`_edges`, `lineage_projection_rebuild_seconds`, `lineage_normalize_failures_total` (reason).

**Maintenance/health**: `maintenance_runs_total` (op,result), `maintenance_duration_seconds` (op), `compaction_conflict_retries_total`★, `fri_debt_fragments`, `table_unindexed_rows` (histogram), `cleanup_reclaimed_bytes_total`, `small_fragments` (histogram).

**Table activity**: `operations_recorded_total` (op_type, source), `index_events_total` (event), `operation_backfill_partial_total` (history gaps from pruned transaction files). Labels are bounded (op_type/event/source) — per-table op logs are `_catalog` data, never metric series.

**Controller lifecycle**: `is_leader`★ (sum must equal 1), `lease_renewals_total` (result), `leader_transitions_total`, `standby_hydration_age_seconds`.

**Caches**: `state_swaps_total`, `cache_ops_total` (cache,result), `cache_bytes` (cache), `cache_spill_bytes`.

**Object store**: `s3_operations_total`★ (op,loop,result — cost attribution per loop), `s3_operation_duration_seconds` (op), `s3_bytes_total` (op,loop), `s3_throttles_total`★ (op). Sourced partly from the lance crate's IO-stat counters.

**Business (namespace-scoped)**: `referenced_bytes` (namespace), `actual_bytes` (namespace), `reclaimable_bytes` (namespace), `tables_total` (namespace,format).

**Runtime/process**: `tokio_*` (busy ratio, queue depth, workers), `process_*` (RSS, CPU, fds, threads) — prove the QoS story.

### 12.4 SLIs & SLOs

| SLO | SLI (sketch) | Target |
|---|---|---|
| Read availability | `1 − rate(http_requests_total{class="metadata",status=~"5.."}) / rate(…{class="metadata"})` | 99.9% |
| Read latency | `histogram_quantile(0.99, http_request_duration_seconds{class="metadata"})` | < 50 ms |
| Freshness | `histogram_quantile(0.99, snapshot_staleness_seconds)` | < sweep cadence, p99 |
| Ingestion liveness | `ingest_inbox_lag_seconds` | < 60 s |
| Governance-data durability | `rate(ingest_deadletter_total)` | ~0 |

Freshness is measured as the observable upper bound — max(inbox lag, per-table time-since-refresh, sweep-position age) — with the sweep guaranteeing the ceiling.

### 12.5 Dashboards (as-code ConfigMaps under `deploy/base/monitoring/dashboards/`)

1. **Overview & SLOs** — RED across all three protocols; availability + latency SLO burn; `is_leader`, pod up/ready, load-shed.
2. **Sync & Convergence** *(signature dashboard)* — inbox lag, dirty-set depth, refresh throughput/latency, coalesce ratio, sweep cycle + position age, snapshot-staleness heatmap, aux-staleness by role, dead-letters. Shows the three loops converging.
3. **Maintenance & Lakehouse Health** — compaction runs/duration/bytes, conflict-retry rate, FRI debt, unindexed-rows heatmap, small-fragment debt, reclaimed vs reclaimable bytes, cost by namespace.
4. **Runtime & S3** — tokio saturation, memory/CPU/fd, object-store ops/latency/bytes/throttles by op, S3 cost attribution by loop.

### 12.6 Alerts (PrometheusRule)

Multi-window burn-rate for SLO alerts; thresholds for failure modes: `ReadAvailabilityBurn`, `ReadLatencyHigh`, `FreshnessBreach`, `InboxLagHigh`, `InboxBacklogGrowing`, `DeadLetterEvents`★, `NoLeader`★ (`sum(is_leader)==0`), `MultipleLeaders`★ (`sum(is_leader)>1`), `SweepStalled`, `CompactionConflictStorm`, `UnindexedRowsHigh`, `S3Throttling`, `HydrationStuck`, `MemoryPressure`, `TargetDown`.

### 12.7 Wiring

ServiceMonitor selects the internal `metrics` port with the operator's release label; PrometheusRule and dashboard ConfigMaps follow the same commit-YAML/operator-picks-it-up model. **Prerequisite**: the cluster runs the Prometheus Operator (kube-prometheus-stack); if present the service just contributes the CRDs, otherwise it's a one-time platform install. **Local parity**: the `local` overlay installs a minimal kube-prometheus-stack into kind (or reuses the platform's), so `tilt up` yields the same dashboards on a laptop and dashboard changes are reviewed as code diffs.

---

## 13. Pillar Review — Validation & Gaps Closed

### Reliability
| Mechanism | Notes |
|---|---|
| S3-only truth, derived caches | No dual-write coherence bugs |
| At-least-once inbox + idempotent apply (domain keys) | Exactly-once *effect*; 24h replay window |
| Three convergent loops | Degraded mode ≡ normal mode, slower |
| Lease election + CAS fencing backstop | Split-brain → commit conflict, not corruption |
| Poison-event handling | Dead-letter prefix + alert |
| DR | Catalog rebuildable by full sweep except lineage/policies → enable S3 versioning on `_catalog/`, optional cross-region replication of `_catalog/` only; restore runbook = reset markers + rehydrate |
| Backpressure | Bounded channels everywhere; shed refresh hints (sweep absorbs), spill lineage, never block ingest |

### Scalability
| Mechanism | Notes |
|---|---|
| Lock-free `ArcSwap` reads, pre-serialized hot payloads | 10k QPS on ~3 small pods |
| Coalescing dirty-set | Load bounds by distinct dirty tables, not event rate |
| Trickle sweep + N+1 HEAD probe | 10k tables ≈ 30 min / ~$6–20/mo request cost |
| S3-as-replication-bus | Read tier scales horizontally, zero coordination |
| QoS isolation of inspection/profiling/MCP-heavy | The real 10k-QPS threat is workload mixing, not volume |
| Known ceiling | ~100k tables → shard refresher/sweeper by table-hash |

### Maintainability
| Mechanism | Notes |
|---|---|
| One manifest set local→prod (kustomize + Tilt + kind) | No compose drift; CI runs the prod shape |
| Cargo workspace: `catalog-core` (state/apply/types), `catalog-store` (object_store + Lance IO), `catalog-api`, `catalog-controller`, `catalog-inspect`, `catalog-mcp` | Apply function is a pure, table-driven unit-test target |
| Spec-pinned OpenAPI + generated models; extensions namespaced on `/ext` | Upstream Namespace/Catalog split lands as a version bump, not a rewrite |
| Observability (§12) | One tower stack for REST/gRPC/MCP metrics; loop-health SLIs; cardinality-governed |
| `_catalog/` schema evolution | `catalog_schema_version` in table config; Lance-native add-column/backfill migrations; startup refuses newer-than-binary schemas |
| Security | OIDC/JWT at tower layer for REST+MCP; per-namespace RBAC in registry properties; read-only enforced in the SQL planner; audit stream for mutations + MCP write tools; native credential vending for data access |
| Testing | Unit: apply-function table tests (dup/reorder/regress). Integration: kind + MinIO in CI incl. conditional-write conformance + kill-the-leader chaos. Load: the §16 QoS proof |

---

## 14. Rejected Alternatives

| Alternative | Rejected because |
|---|---|
| Postgres metadata store | S3 was always truth (commits bypass catalog); PG adds reconciliation + a standing lie about authority |
| Kafka / SQS / EventBridge / NATS | Trivial volume; level-triggered idempotent refresh needs no ordered durable log; S3 inbox preserves the single-stateful-dependency invariant |
| Redis / external cache | 100–300MB fits in heap; single writer; restart solved by checkpoints; would reintroduce multi-copy coherence |
| S3 event notifications | Two managed services delivering pointers a $13/mo LIST-tail covers; breaks S3-API portability (MinIO local dev) |
| Generic catalog (Polaris Generic Table, etc.) | Naming/discovery only — no Lance-aware maintenance, index staleness, aux grouping, lineage-aware TTL |
| actix-web | 10–15% raw throughput irrelevant at this envelope; loses tonic/tower/rmcp single-stack alignment |
| Python sidecar for stats/profiling | `lance` Rust crate exposes statistics/optimize natively; DataFusion covers profiling — one language, one binary |
| Enriched standard `ListTables` | Spec `ListTables` is names-only; enrichment must be an `/ext` endpoint |
| Separate metrics/cache tiers, OTel collector | Pull-based `/metrics` + in-heap state need none; keeps the stack lean |
| docker-compose for local | Second manifest lineage that drifts; kind + kustomize + Tilt runs the real thing |

## 15. Known Scaling Ceiling

Design comfortably covers ~10k tables and 10k QPS. At **~100k+ tables**, sweep time and checkpoint size grow linearly enough to shard the refresher/sweeper by table-hash. Stated as a known limit, not solved preemptively.

## 16. Open Items / Risks

1. **Load-test before finalizing pod counts** — metadata-read p99 while the DataFusion pool is saturated decides whether `catalog-inspect` is day-one.
2. **MinIO conditional-write parity** — pin version, keep the CI conformance test green; fallback LocalStack.
3. **Namespace spec drift** — track the upstream Namespace/Catalog split; generate models from the pinned `spec.yaml` and diff each bump; extensions stay on `/ext` so re-alignment is a version bump. Also re-verify `ListAllTables`/`QueryTable` shapes and the full numeric error enum against the pinned spec before freezing the API surface.
4. **OL event fidelity from Spark** — verify lance-spark + OL listener emits the output version/facets needed for `parent_version_at_sync`; if absent, contribute upstream or carry a thin custom facet.
5. **Profiling cost governance** — sampling caps + per-namespace budgets before GA on huge multimodal tables.
6. **FRI remap scheduling** — FRI defers work; without a periodic remap policy, read amplification accumulates.
7. **Cardinality governance** — table-scoped facts are API/`_catalog` queries, never Prometheus labels (enforced in review).
8. **Operation-history module** — `lance-history` (§5.10) is vendored until upstreamed; track the Lance PR and drop the vendored copy on merge. Operation history is `partial` for versions whose transaction files were pruned before the catalog first observed the table (durable going forward); tables should ideally be registered at creation so history is complete from v1.

---

## v1.0.0 as-built notes

The following notes record where the v1.0.0 implementation concretizes or
deviates from the design above. The design is the long-term vision; v1.0.0
made specific choices for the shipped subset. These notes are appended
rather than rewriting the original design intent.

### Version management (§5.6) — timestamp-path snapshots, not Lance-native versions

Design §5.6 assumed Lance-native manifest versions (tag CRUD, `RestoreTable`,
schema diff, etc. backed by Lance history). v1.0.0 instead models versions
as **timestamp-path snapshot siblings**: each version is a directory
`<table>/<YYYY-MM-DD_HH-MM-SS>/` (or `<table>/<YYYY-MM-DD-HH-MM-SS>/` — name
variance handled) containing an independent Lance dataset copy plus its aux
directories. The version id is the ISO8601 normalization of the parsed
timestamp. There is no Lance-manifest-level versioning relationship between
siblings — each timestamp directory is an independent snapshot. Tag CRUD,
`RestoreTable`, and schema-diff-between-versions are not implemented in
v1.0.0 (deferred).

### Pre+post 2026-06-26 cutoff aux layout

Design §3 assumed a single storage layout. v1.0.0 supports two real-world
layouts that differ by a cutoff around 2026-06-26:

- **Pre-cutoff** (< 2026-06-26): sidecar directories
  (`_FragmentMetadata/`, `master_indices/`, `lance_tags/`,
  `lance_tags_intermediate/`, `_asset_replication_segments/`,
  `_asset_replication_results/`, `curated_indices/`) live **inside**
  `dataset.lance/`, alongside the lance-core directories (`_versions/`,
  `_transactions/`, `_indices/`, `data/`). This inflates `dataset.lance/`'s
  size with sidecar content.
- **Post-cutoff** (>= 2026-06-26): sidecar directories move to a top-level
  `dataset.sidecar/`; `dataset.lance/` is clean (lance-core only).
- **Transition** (~2026-06-12): sidecar content is duplicated both inside
  `dataset.lance/` and at top-level `dataset.sidecar/` (dual-write). The
  sweep deduplicates: when a top-level `dataset.sidecar/` is present it is
  preferred; otherwise sidecar bytes are read from inside `dataset.lance/`.

The main lance directory name also varies: `dataset.lance/` for most tables,
`dataset/` for some. v1.0.0 detects the main lance directory by the presence
of `_versions/` + `_transactions/` at its root, not by name.

Aux directory formats are arbitrary (parquet, lance, csv, mixed, unknown) —
the format is detected per-directory, and the directory name is recorded as
the `role` with no ontology layer in v1.0.0. The storage byte totals are
split per-component (`lance_core_bytes`, `sidecar_bytes`, `segments_bytes`,
`other_aux_bytes`) so pre-cutoff versions attribute sidecar bytes correctly
rather than lumping them into lance-core.

### Real-world example tables (the surveyed layout)

The sweep root `s3://onroad-perception-datasets/scenario_dataset_export/`
(the configurable default) contains ~65 table directories. The
representative tables below are the real storage layouts the sweep is
designed against. Aux directory names and formats vary per table, so
classification is structural (by directory contents), not name-based.

An annotated example tree for `smoke_test` showing both cutoff layouts:

```
scenario_dataset_export/
  smoke_test/
    2026-06-27_01-37-36/            # a post-cutoff version (current layout)
      dataset.lance/                # main lance dataset (_versions/ + _transactions/ at root)
        _versions/  _transactions/  _indices/  data/
      dataset.sidecar/              # top-level sidecar (post-cutoff placement)
        _FragmentMetadata/  master_indices/  lance_tags/  lance_tags_intermediate/
        _asset_replication_segments/  _asset_replication_results/  curated_indices/
      segments/                     # parquet aux (_SUCCESS + part-*.snappy.parquet)
    2026-06-25_13-42-42/            # a pre-cutoff version (old layout)
      dataset.lance/                # sidecar dirs live INSIDE here (inflating lance-core size)
        _versions/  _transactions/  _indices/  data/
        _FragmentMetadata/  master_indices/  lance_tags/  lance_tags_intermediate/
        _asset_replication_segments/  _asset_replication_results/  curated_indices/
      segments/
```

The 2026-06-26 boundary is observable in `smoke_test`: the 2026-06-25 version
carries sidecar directories inside `dataset.lance/` (old layout), while the
2026-06-27 version has a clean `dataset.lance/` plus a top-level
`dataset.sidecar/` (current layout).

The representative tables and their characteristics:

| Table | Versions | Notable characteristics |
|---|---|---|
| `smoke_test` | 457 | Spans the cutoff cleanly; exercises the full range of version shapes across its history. |
| `closed_loop_run_purpose_dataset` | 173 | Clean cutoff boundary; carries a `-MISSING-RECONSTRUCTIONS` sibling partial. |
| `1stage_scenario_dataset_train` | 65 | Post-cutoff versions can be `lance_only_partial` — a `dataset.lance/` containing only nested `index_datasets/` + `tag_datasets/` lances with no `_versions/`/`_transactions/` at its own root. Also uses the hyphen timestamp format (`YYYY-MM-DD-HH-MM-SS`). |
| `1stage_scenario_dataset_eval` | 39 | Crosses the cutoff and shows the ~2026-06-12 transition dual-write (sidecar both inside `dataset.lance/` and at top-level `dataset.sidecar/`). Aux variety includes nested lance (`scenario_dataset_etl/`, `single_segment.lance/`) and partitioned parquet (`entity_asset_replication_result/`). |
| `robotaxi` | 10 | Mostly pre-cutoff; aux variety includes CSV (`curated_csv/`, `row_counts/`), demonstrating arbitrary-aux-format handling. |

These tables exercise every sweep code path: the pre/post cutoff layouts, the
dual-write transition, the `lance_only_partial` nested-lance edge case, the
hyphen timestamp variance, and the arbitrary aux formats (parquet, lance, csv,
mixed).

### Retention / TTL (§5.7) — per-table API policy, hard delete

Design §5.7 described a TTL engine with native-knob-first config writes,
lineage-aware retention, and reclaimable-bytes/rollback-horizon dry-run.
v1.0.0 concretizes this to a per-table API policy with hard-delete semantics:

- The TTL policy is set per table via the REST API
  (`PUT /v1/table/:id` with `ttl_policy`), not via Lance table config
  (`lance.auto_cleanup.*` targets Lance-native versions, not timestamp-path
  snapshots).
- A version is eligible only if it fails *both* set thresholds
  (`keep_last_n` AND `max_age_days`; within either => kept), is not
  `protected`, and passes the shape safety gate (`full`/`lance_only`/
  `seg_only` only; `lance_only_partial`/`empty` refused).
- `apply` hard-deletes the entire `<table>/<timestamp>/` prefix tree from S3
  (irreversible), appends a `TtlAuditRecord` per deletion to
  `_catalog/ttl_audit` (durable Lance `Append`) **before** removing the
  version from the registry, so a mid-apply crash can leave a duplicate audit
  entry but never a lost one. A `snapshot_path` that does not resolve under
  the sweep root is refused rather than fake-succeeding the delete.
  Lineage-aware retention (never reap a version referenced as a job input)
  is not implemented in v1.0.0 (deferred alongside lineage).
- Dry-run returns candidate versions + `reclaimable_bytes` (logical/deduped
  size, not physical footprint — see the TTL runbook for the caveat).
- `protected` flag exempts a version from TTL (API-set).

The full TTL safety guidance (including the non-atomic audit-vs-registry
write caveat and the partial-failure no-rollback behavior) is in
[`docs/RUNBOOK-TTL.md`](RUNBOOK-TTL.md).
