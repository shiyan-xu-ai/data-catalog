---
title: SQL Workload Limits and Result Cache - Plan
type: feat
date: 2026-11-19
topic: sql-workload-limits
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# SQL Workload Limits and Result Cache - Plan

## Goal Capsule

- **Objective:** About 100 users, up to 10 at once, can run arbitrary SQL in production without any one query restarting the service or stalling catalog browsing for others, and repeated queries return quickly.
- **Means:** Harden the single catalog service: isolate query execution, budget its memory, admit queries per user and globally, cancel on the server, and cache results by table version.
- **Product authority:** The Product Contract below. Queueing, scale-out on load, and a separate SQL service are not active scope.
- **Open blockers:** None.

---

## Product Contract

### Summary

The catalog service runs SQL on its own thread pool inside a fixed memory budget, sized for ten concurrent users. A query starts only when a global slot is free and its user has fewer than two queries running; otherwise it is rejected at once with a message that names the limit. Cancel stops the query on the server. Identical queries over unchanged table versions are answered from an in-memory cache shared by all users.

### Problem Frame

Production will have about 100 users and up to 10 concurrent ones running ad hoc SQL and analysis. The service runs on 1 vCPU and 1 GiB (`project.toml`), and SQL shares one process and one thread pool with catalog browsing. Each query may use a 512 MB memory pool and two may run per instance, which already equals the instance's memory; DataFusion's pool also does not count scan buffers or the result being built. The largest table, `automotive.scenarios.1stage_scenario_dataset_v2`, holds 8.4M rows and about 1 GB of column data in 4,713 fragments (its 19.9 TB of blobs are never read by SQL). A full scan, group-by, or join over it can restart the instance, failing every in-flight request, or occupy the CPU so browsing stalls. Cancel only stops the browser waiting, so an abandoned query keeps its resources. A repeated query pays the full scan again.

### Key Decisions

- **Reject when full; no queue.** (session-settled: user-directed — chosen over a bounded wait queue and over automatic scale-out: simpler, and users retry by hand.) Governs R4, R5.
- **Up to two running queries per user.** (session-settled: user-directed — chosen over one per user and over no per-user limit: one user cannot hold every slot, while two still allows a quick query beside a slow one.) Governs R4, R6.
- **One hardened service, not a separate SQL service.** (session-settled: user-directed — chosen over a second Apps Platform deploy for SQL only: one deploy and one origin; a rare runaway can still restart one instance.) Governs R1, R2, R3.
- **In-memory result cache per instance, shared by all users.** (session-settled: user-directed — chosen over a Postgres-backed cache and over per-user caches: simplest and fastest; no per-user data access exists today.) Governs R11, R12, R13.
- **Cancel stops the query on the server.** (session-settled: user-directed — chosen over keeping client-only cancel: otherwise a cancelled query holds its owner's slot until the time limit.) Governs R7.
- **Hard time limit of 300 seconds.** (session-settled: user-directed — chosen over the current 60 s and a proposed 30 s.) Governs R8.

### Requirements

**Isolation and memory**

- R1. Query execution runs on a thread pool separate from HTTP request handling, so catalog browsing stays responsive while queries run.
- R2. All running queries together stay within a fixed share of the instance's memory, with each query's own limit carved from that share and headroom kept for memory the query engine does not track.
- R3. The production instance size, global slot count, and memory share are set so ten concurrent users running heavy queries cause no instance restarts; the values are chosen from the load test in the Success Criteria, and stay configurable.

**Admission**

- R4. A query starts only when a global slot is free and its user has fewer than two queries running; otherwise it is rejected immediately, without queuing.
- R5. A rejection says which limit was hit: the server is busy, or the user already has two queries running.
- R6. A user is identified by the signed-in identity the platform's IAP puts on each request; a request without one counts as one shared anonymous user.

**Lifetime and size**

- R7. Cancelling a query in the UI, closing its tab, or leaving the page stops it on the server and frees its slot right away.
- R8. Every query has a hard time limit of 300 seconds, configurable.
- R9. A result is capped by both rows (today 1,000) and encoded bytes; a query over either cap returns what fits, flagged as truncated, and the UI says which cap applied.
- R10. Existing guarantees stay: queries are read-only, blob payloads are never read, and a query over its memory limit fails with a clear error rather than taking the instance down.

**Result cache**

- R11. A query whose text and resolved table versions match a cached result returns that result without reading table data.
- R12. The cache lives in each instance's memory, is shared by all users on that instance, and is bounded in size, evicting the least recently used entries.
- R13. The results footer marks a cached result as cached; a cached result is never stale, because a table version never changes once written.

### Acceptance Examples

- AE1. **Covers R4, R5.** Given a user with two queries running, when they start a third, it is rejected at once with a message that they already have two running; another user's query still starts if a global slot is free.
- AE2. **Covers R4, R5.** Given every global slot busy, when any user starts a query, it is rejected at once with a "server busy, try again" message.
- AE3. **Covers R7.** Given a user's query has run for 20 seconds, when they press Cancel, the server stops it and their next query starts without a per-user rejection.
- AE4. **Covers R11, R13.** Given a query ran over `"prod"."events"` at version 12, when anyone runs the same text while the current version is still 12, the cached result returns without a table scan and is marked cached; after a refresh moves the current version to 13, the same text scans again.
- AE5. **Covers R9.** When a query without a LIMIT selects every row of the 1stage scenario table, the first rows that fit the row and byte caps return, flagged as truncated.

### Success Criteria

- A load test on a staging deploy, with ten concurrent users running full-table group-bys and joins over the 1stage scenario table, causes no instance restarts, and catalog browsing requests stay responsive throughout.
- A simple query (a filtered or limited select) returns within a few seconds, and a large query typically within 30 seconds, under that load.
- A repeated identical query over unchanged tables returns in well under a second.

### Scope Boundaries

**Deferred for later**

- Queueing queries when slots are full, and scaling out on query load.
- A separate SQL service or worker pool.
- A result cache shared across instances or surviving restarts.
- Quotas over time, such as a daily budget per user.
- Exporting or downloading results beyond the caps.

**Outside this work**

- Per-user data access control.

### Dependencies / Assumptions

- The platform's IAP signs every production request with the user's identity in a request header; the service does not read it today (`README.md`: "The service has no authentication of its own").
- Table versions are immutable once committed, so a cache keyed by resolved versions cannot serve stale data (`ARCHITECTURE.md`, "Current version and caching").
- Today's limits are config: `CATALOG_SQL_MAX_ROWS`, `CATALOG_SQL_TIMEOUT_SECS`, `CATALOG_SQL_MEMORY_LIMIT_MB`, `CATALOG_SQL_MAX_CONCURRENT` (`catalog-api/src/config.rs`); the instance is 1 vCPU / 1 GiB (`project.toml`).
- DataFusion's memory pool limits only large operators such as sorts, joins, and group-bys, not scan buffers or batches in flight (`datafusion-execution` 54, `memory_pool/mod.rs`), and it recommends a separate runtime for CPU-bound query work (`datafusion` 54, `lib.rs`).

### Sources / Research

- `catalog-store/src/sql.rs` (`session`, `execute`): per-query session, memory pool, row cap, and time limit.
- `catalog-api/src/rest.rs` (`run_sql`, `sql_slots`): the per-instance semaphore and 429 throttling.
- `catalog-api/src/config.rs` (`SqlConfig`) and `project.toml` (`[cloudrun]`): current limits and instance size.
- The largest table's size came from the running catalog: `count_rows` (8,412,721), `fragment-measures` (4,713 fragments), and `storage` (1.03 GB data, 19.9 TB blob).

---

## Planning Contract

Product Contract unchanged; its deferred questions are resolved in KTD3, KTD6, and KTD7. One pull request.

### Key Technical Decisions

- KTD1. **Queries run on a dedicated Tokio runtime.** The whole query (open tables, plan, execute, encode) is spawned onto a separate multi-thread runtime with `CATALOG_SQL_THREADS` workers; the request handler awaits it. DataFusion's own guidance is a separate runtime for CPU-bound plans (`datafusion` 54, `lib.rs`). Governs R1.
- KTD2. **Static memory partition.** `CATALOG_SQL_MEMORY_BUDGET_MB` is the total for all queries; each query's pool is the budget divided by `CATALOG_SQL_MAX_CONCURRENT`. This replaces `CATALOG_SQL_MEMORY_LIMIT_MB`. The budget is set well below the instance memory because the pool does not see scans or results being built. Governs R2, R10.
- KTD3. **Explicit cancel plus abort on drop.** The client sends a query id it generates; a registry maps running ids to their owner and task. `POST /catalog/v1/sql/:query_id/cancel` aborts the caller's own query. The handler also aborts its task when its future is dropped (client disconnect), and the page sends a keep-alive cancel request when it unloads. Governs R7.
- KTD4. **Admission registry replaces the semaphore.** One lock-guarded structure counts running queries globally and per user; `CATALOG_SQL_MAX_PER_USER` (2) is the per-user cap. Rejections are Throttling (429) with distinct messages. Governs R4, R5.
- KTD5. **Identity from the IAP header.** The user is the value of `X-Goog-Authenticated-User-Email` (Google IAP's documented header) with its `accounts.google.com:` prefix removed; absent means `anonymous`. The header is trusted only because IAP fronts every production request. Each admission decision logs the resolved identity, so a deploy where every request resolves to `anonymous` is visible. Governs R6.
- KTD6. **Result cache keyed by text and versions.** Key: the SQL text with surrounding whitespace trimmed, plus the sorted (table id, version) pairs the statement resolved to. Tables are resolved (cheap, metadata only) before the lookup; a hit skips execution. `moka` future cache weighted by encoded bytes, `CATALOG_SQL_CACHE_MB` (64). Only complete, successful results are cached, and never one whose plan calls a volatile or stable function (such as `now()` or `random()`), an `EXPLAIN` statement, or a query that read no table, because those depend on more than the table versions. The response gains `cached`. Governs R11, R12, R13.
- KTD7. **Byte cap during encoding.** Rows are encoded until `CATALOG_SQL_MAX_RESULT_MB` (8) would be exceeded; the response's `truncated_by` is `rows`, `bytes`, or null. Governs R9.
- KTD8. **Defaults and production sizing.** `CATALOG_SQL_TIMEOUT_SECS` default 300. `project.toml` moves to 4 vCPU / 8 GiB with `CATALOG_SQL_MAX_CONCURRENT=10`, `CATALOG_SQL_MAX_PER_USER=2`, `CATALOG_SQL_THREADS=3`, `CATALOG_SQL_MEMORY_BUDGET_MB=3072` (about 300 MB per query). A local memory-limited load test is the pre-flight; the Success Criteria's staging load test is the gate before rollout. Governs R3, R8.

### Risks & Dependencies

- Whether Cloud Run propagates a browser disconnect to the server is unverified; the explicit cancel request (KTD3) does not depend on it.
- Registry, slots, and cache are per instance. With more than one instance, a cancel request can land on an instance that is not running the query (it returns 404 and the query runs to its limit), and a user can run two queries on each instance. Capping the service at one instance, if Apps Platform exposes that setting, makes both exact; the client treats a cancel 404 as unconfirmed rather than as success.
- Cloud Run's default request timeout is also 300 s, so a query near the limit may be cut off by the platform with a generic gateway error; raise the platform timeout above the SQL limit if Apps Platform exposes it.
- Lance decodes data on its own CPU pool, outside the dedicated query runtime, so browsing routes that decode through Lance still compete with queries; the load test decides whether that matters.
- A cache hit still opens table metadata to resolve versions; that is cached per version (`ARCHITECTURE.md`), so it is cheap after the first query.
- Changing `CATALOG_SQL_MEMORY_LIMIT_MB` to a budget variable is a config break for any deploy that set it; `project.toml` does not.

---

## Implementation Units

### U1. Query runtime, registry, and cancellation

- **Goal:** Queries run off the request runtime, can be cancelled by id, and stop when the request goes away.
- **Requirements:** R1, R7. KTD1, KTD3.
- **Dependencies:** none.
- **Files:** `catalog-api/src/rest.rs`, `catalog-api/src/config.rs`, `catalog-api/src/main.rs`, `catalog-api/src/sql_runner.rs` (new), `catalog-api/tests/rest.rs`.
- **Approach:**
  1. Build the query runtime at startup from config and hand its handle to the router state.
  2. `run_sql` takes an optional `query_id` in the body, registers it with the caller's identity, spawns the query on the query runtime, and holds an abort-on-drop guard on the task.
  3. Add `POST /catalog/v1/sql/:query_id/cancel`; it aborts only a query the same user owns and returns 204, or 404 when no such query runs.
  4. Deregister on every exit path, including abort.
- **Test scenarios:**
  - A query cancelled through the endpoint returns promptly with a cancelled error, and its id is gone from the registry.
  - Cancelling another user's query id returns 404 and leaves it running.
  - Dropping the request future aborts the spawned query (unit test on the guard).
- **Verification:** `cargo test -p catalog-api` passes.

### U2. Admission and identity

- **Goal:** Global and per-user caps with distinct rejection messages.
- **Requirements:** R4, R5, R6. KTD4, KTD5.
- **Dependencies:** U1.
- **Files:** `catalog-api/src/sql_runner.rs`, `catalog-api/src/rest.rs`, `catalog-api/src/config.rs`, `catalog-api/tests/rest.rs`.
- **Approach:** Admission happens before spawning; a slot is held by a guard released on completion, error, or abort.
- **Test scenarios:**
  - Covers AE1. With per-user cap 2, a third concurrent query from one user is 429 "you already have 2 queries running", while another user's query is admitted.
  - Covers AE2. With every global slot held, any user gets 429 "server busy".
  - A cancelled query frees its slot for the next request (AE3).
  - Requests without the IAP header share the `anonymous` user.
- **Verification:** `cargo test -p catalog-api` passes.

### U3. Memory budget, byte cap, and time limit

- **Goal:** Per-query pools carved from one budget, results bounded by bytes, 300 s default.
- **Requirements:** R2, R8, R9, R10. KTD2, KTD7, KTD8.
- **Dependencies:** none.
- **Files:** `catalog-store/src/sql.rs`, `catalog-api/src/config.rs`, `catalog-store/tests/sql.rs`, `catalog-api/tests/rest.rs`.
- **Approach:** `SqlLimits` carries the per-query memory and the byte cap; `encode_rows` stops at the byte cap and reports which cap truncated.
- **Test scenarios:**
  - Covers AE5. A result over the byte cap returns only rows that fit, with `truncated_by: "bytes"`.
  - A result over the row cap reports `truncated_by: "rows"`; a small result reports null.
  - Config rejects a budget smaller than one megabyte per slot.
- **Verification:** `cargo test --workspace` passes.

### U4. Result cache

- **Goal:** Repeat queries over unchanged versions skip execution.
- **Requirements:** R11, R12, R13. KTD6.
- **Dependencies:** U3.
- **Files:** `catalog-store/src/sql.rs`, `catalog-store/Cargo.toml`, `Cargo.toml`, `catalog-api/src/config.rs`, `catalog-store/tests/sql.rs`.
- **Approach:** Split `query_sql` into resolve (parse, rewrite, open) and execute; look up between them; insert complete results only.
- **Test scenarios:**
  - Covers AE4. The same text twice returns `cached: true` the second time; after the table gains a version and is refreshed, the same text misses.
  - `SELECT now()` and a table query filtered by `now()` are never cached.
  - A failed query is not cached.
  - Entries beyond the byte budget are evicted.
- **Verification:** `cargo test -p catalog-store` passes.

### U5. Client: cancel, unload, cached and truncation display

- **Goal:** The UI uses query ids, cancels on the server, and shows cached and truncation reasons and the rejection message.
- **Requirements:** R5, R7, R9, R13.
- **Dependencies:** U1, U3, U4.
- **Files:** `frontend/src/lib/api.ts`, `frontend/src/lib/schemas.ts`, `frontend/src/components/query/query-pane.tsx`, `frontend/src/components/query/query-page.tsx`, `frontend/src/components/query/result-grid.tsx`, `frontend/src/lib/schemas.test.ts`, `frontend/src/lib/api.test.ts`.
- **Approach:** Generate the id with `crypto.randomUUID()`; Cancel and closing a tab call the cancel endpoint; `pagehide` sends `fetch(..., {keepalive: true})` for each running id; show the server's 429 message as is; footer shows "cached" and "truncated by rows/bytes".
- **Test scenarios:**
  - The response schema accepts `cached` and `truncated_by`.
  - Browser smoke: Cancel frees the slot (a new query runs immediately); a repeated query shows "cached".
- **Verification:** `bun run test` and `bun run build` pass.

### U6. Sizing, docs, and load test

- **Goal:** Production config and docs match, and a load test backs the numbers.
- **Requirements:** R3. KTD8.
- **Dependencies:** U1–U5.
- **Files:** `project.toml`, `README.md`, `ARCHITECTURE.md`, `docs/FRONTEND.md`.
- **Approach:** Update the env table and the SQL route row (cancel route, `query_id`, `cached`, `truncated_by`, 429 messages). As pre-flight, run ten concurrent heavy queries against the 1stage scenario table on a local server limited to the planned memory, and record peak memory and browsing latency in the PR description. The staging load test in the Success Criteria, including a check that two signed-in users resolve to two identities, is an operator step before rollout.
- **Test expectation:** none -- config and docs; the load test is the verification.
- **Verification:** The local pre-flight shows no out-of-memory, zero "server busy" rejections at ten users, and browsing responsive.

---

## Verification Contract

| Check | Command |
|---|---|
| Rust | `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `CATALOG_TEST_DATABASE_URL=$DATABASE_URL CATALOG_REQUIRE_DB_TESTS=1 cargo test --workspace --all-targets` |
| Frontend | `cd frontend && bun run test && bun run build` |
| Load (pre-flight) | Ten concurrent heavy queries on the 1stage scenario table against a memory-limited local server; the staging run is the rollout gate |

## Definition of Done

- R1–R13 and AE1–AE5 hold; all Verification Contract checks pass.
- `CATALOG_SQL_MEMORY_LIMIT_MB` and the semaphore are gone; README env table and route rows match.
- No scaffolding or abandoned code remains.
