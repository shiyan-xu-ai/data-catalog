# TTL safety runbook

The TTL engine hard-deletes old timestamp version directories from S3. This is
an **irreversible, destructive** operation. This runbook documents the
policy semantics, the mandatory workflow, and the known caveats an operator
must understand before running an apply.

## Policy semantics

Each table carries an optional per-table `TtlPolicy` (set via
`PUT /v1/table/:id` with a `ttl_policy` body):

```json
{"ttl_policy": {"keep_last_n": 10, "max_age_days": 90}}
```

- `keep_last_n` (optional int): keep the N most recent versions (by
  `timestamp`, descending).
- `max_age_days` (optional int): keep versions younger than this threshold
  (`now - timestamp` <= `max_age_days`).
- **If neither is set (no policy), nothing is ever eligible.** This is the
  safe default — a table with no policy is never touched by the TTL engine.
- **AND-logic when both are set:** a version is eligible only if it fails
  *both* thresholds — i.e. it is beyond the keep-count AND older than the age
  threshold. A version that is within *either* threshold is kept. (So
  `keep_last_n=10, max_age_days=90` deletes a version only if it is both older
  than 90 days AND not among the 10 most recent.)
- If only one threshold is set, only that one constrains; the other is
  ignored.

### Protected exemption

A `protected` boolean on a version (`PUT /ext/v1/tables/:id/versions/:vid/protect`
with `{"protected": true}`) exempts that version from TTL *regardless* of the
policy. Protected versions are never eligible. The apply path additionally
defends against this in code: even if a protected version somehow appeared in
the eligible set, the per-version delete loop skips it as the last line of
defense before the irreversible S3 delete.

### Shape safety gate

The TTL engine refuses to delete versions whose shape is not cleanly
classifiable. Only `full`, `lance_only`, and `seg_only` versions are
deletable. `lance_only_partial` and `empty` are refused outright — the engine
cannot be confident about what exactly it would delete for a version whose
shape wasn't cleanly classified.

## Mandatory workflow: dry-run before apply

**Never run `POST .../ttl/apply` without first running
`GET .../ttl/dryrun` for the same table and reviewing the candidate list.**

1. **Dry-run** (read-only, any pod):

   ```sh
   curl -s "http://localhost:8080/ext/v1/tables/<id>/ttl/dryrun" | python3 -m json.tool
   ```

   Response:

   ```json
   {"table_id": "<id>", "candidates": ["2024-01-01T00:00:00", ...], "reclaimable_bytes": 12345}
   ```

   Review `candidates`. These are the version ids that *will* be hard-deleted
   by an apply under the table's current policy, as of now. If the list is
   empty, an apply is a no-op.

2. **Apply** (leader-only, irreversible):

   ```sh
   curl -s -X POST "http://localhost:8080/ext/v1/tables/<id>/ttl/apply" | python3 -m json.tool
   ```

   Response:

   ```json
   {"table_id": "<id>", "deleted": ["2024-01-01T00:00:00", ...], "reclaimed_bytes": 12345}
   ```

   `deleted` is the list of version ids actually hard-deleted by this call.

### Apply is leader-only

A non-leader pod returns `503 Service Unavailable` for `POST .../ttl/apply`.
Clients retry; any pod behind the same Service will eventually hit the leader.
There is no cross-pod forwarding in v1.0.0.

### Apply recomputes eligibility fresh

The apply endpoint never trusts the dry-run response the caller might be
holding. It recomputes eligibility against the current registry state inside
the `write_lock`-guarded critical section, so a stale dry-run cannot cause an
unintended deletion. Re-applying after a successful apply is a no-op: a
version already removed from the registry is not recomputed as eligible.

### Idempotent re-apply

TTL apply is idempotent by design. If a previous apply deleted some versions
and was then interrupted, re-running apply will only re-attempt the remaining
eligible versions. A version already deleted from S3 (and removed from the
registry) is not recomputed as eligible, so it is not re-attempted.

## Reading the audit log

Every successful apply appends one `TtlAuditRecord` per deleted version to
the `_catalog/ttl_audit` Lance table. Read a table's audit log via:

```sh
curl -s "http://localhost:8080/ext/v1/tables/<id>/ttl/audit" | python3 -m json.tool
```

Response is a JSON array of records:

```json
[
  {
    "table_id": "<id>",
    "version_id": "2024-01-01T00:00:00",
    "deleted_at": "2026-07-04T12:34:56.789Z",
    "reclaimed_bytes": 12345,
    "policy_snapshot": {"keep_last_n": 10, "max_age_days": 90},
    "actor": "ttl-engine"
  },
  ...
]
```

- `deleted_at` is an RFC3339 timestamp.
- `reclaimed_bytes` is the `storage_bytes_total` (logical/deduped size) the
  version had at delete time — see the caveat below.
- `policy_snapshot` is the `TtlPolicy` that was in effect when the apply ran.
- `actor` is the apply principal (`ttl-engine` for the automated engine).

If no TTL deletion has ever been recorded for the table, or the audit dataset
does not exist yet, the endpoint returns an empty list (not an error). An
empty list therefore means "no deletions recorded", never "the audit log could
not be read": a genuine read failure (transient object-store error, corrupt
manifest) now returns `500`, so an empty response can be trusted.

## Shielding a version with `protected`

To prevent a specific version from ever being deleted by TTL (e.g. a
production-pinned snapshot), set its `protected` flag:

```sh
curl -s -X PUT "http://localhost:8080/ext/v1/tables/<id>/versions/<vid>/protect" \
  -H 'Content-Type: application/json' \
  -d '{"protected": true}' | python3 -m json.tool
```

This is a leader-only write. Protected versions are excluded from eligibility
in dry-run and apply, and are additionally skipped as a last line of defense
in the delete loop. Clear with `{"protected": false}`.

## Known caveats

These are documented in-scope behaviors an operator must understand. They are
not bugs; they are inherent to the v1.0.0 design and are called out here so
they are not surprising during an incident or audit.

### (a) `reclaimable_bytes` is logical/deduped size, not physical footprint

`reclaimable_bytes` (in both the dry-run response and the audit record's
`reclaimed_bytes`) is the sum of `storage_bytes_total` over eligible/deleted
versions. `storage_bytes_total` is a *logical* (deduped) size — the sum of the
per-component byte totals (lance_core + sidecar + segments + other_aux),
after dedup of sidecar content that exists in both places during the
~2026-06-12 dual-write transition (top-level `dataset.sidecar/` wins; the
duplicate inside `dataset.lance/` is not double-counted).

This means `reclaimable_bytes` may slightly **under-count the physical bytes
freed** when the whole `<table>/<timestamp>/` tree is hard-deleted, especially
for versions that span the dual-write transition (where the physical tree
contains both the top-level and the in-lance copy of sidecar content). It is
a correct measure of the catalog's logical accounting of the version's size,
not of the S3 physical footprint freed by the delete.

Additionally, loose files directly at the main-lance-dir root (not inside
`_versions/`, `_transactions/`, `_indices/data`, or a recognized subdir) are
not summed into any byte total (only subdirectories are recursed). This is a
~zero undercount in practice for standard Lance 8.0.0 layouts, but is a latent
undercount if a future layout drops a file at the dataset root.

### (b) The TTL apply audit-vs-registry write is not atomic across the two Lance tables

`ttl_apply` does, per eligible version: delete the S3 prefix and collect an
audit record for it. Then *after* the loop, in order: `append_ttl_audit` (the
audit write, a Lance `Append`) **first**, then `persist_and_refresh_cache`
(the registry write, a Lance `Overwrite`) that removes the deleted versions.
These are two separate non-atomic Lance writes.

The audit is written **before** the registry removal deliberately. If the
process crashes between the two writes, the deletions are already recorded in
the durable audit log, but the registry still lists the (already-deleted)
versions. That is self-healing: the next apply recomputes those versions as
eligible, re-issues the delete (a NotFound no-op — the objects are already
gone), removes them from the registry, and appends a **duplicate** audit
record. So the residual failure mode is a possible *duplicate* audit entry on
crash + retry, never a *lost* one.

This ordering is the deliberate safer choice: the audit log is the only
durable evidence that an irreversible hard-delete happened, so audit
completeness is prioritized over audit dedup. (The audit write also uses Lance
`Append`, not a read-all-then-`Overwrite` rewrite, so a transient read error
can never truncate prior audit history.) True atomicity across the two Lance
tables would need a transaction spanning both, which is out of scope for
v1.0.0.

**Operational implication:** the audit log never *under*-records a completed
delete, but may contain a duplicate record for a version whose apply was
interrupted and retried. When reconciling, de-duplicate audit records by
`(table_id, version_id)`. The registry remains the source of truth for "what
versions exist"; the audit log is the durable history of deletions.

### (c) Partial-failure behavior (no rollback, by design)

If a deletion errors partway through the eligible list (e.g. a transient
object-store error on one version's prefix), versions successfully deleted
*before* the error are still persisted (removed from the registry, audited) —
they are **not** rolled back. An S3 delete cannot be un-done, so recording
what actually happened is safer than pretending the whole call failed
atomically.

In the partial-failure case the apply endpoint returns a `500` naming which
version(s) failed. The caller can retry the apply, which will only re-attempt
the remaining eligible versions (already-deleted versions are no longer
eligible and are skipped). There is no rollback path; this is by design.
