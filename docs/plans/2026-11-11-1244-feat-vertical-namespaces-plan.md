---
title: Vertical Namespace Layout - Plan
type: feat
date: 2026-11-11
topic: vertical-namespaces
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Vertical Namespace Layout - Plan

## Goal Capsule

- **Objective:** A catalog user can open a business vertical (automotive, trucking, mining, robotics) and find every dataset that belongs to it, organized by use-case domain instead of by S3 storage layout.
- **Means:** Organize the catalog as `<vertical>/<domain>/<table>`, with the top-level namespace restricted to a configured vertical allowlist; move the three registered tables into that layout; make the web UI read-only.
- **Product authority:** This Product Contract wins on behavior; the Planning Contract wins on mechanism within it. Cross-vertical tagging and table moves that preserve history are not active scope.
- **Execution profile:** One PR. Running the move against the deployed catalog is an operator step after deploy, not part of the PR.
- **Stop conditions:** Stop if enforcing the allowlist would break reads of existing namespaces (R4), or if the move cannot carry owner, description, and properties (R6).
- **Open blockers:** None.

---

## Product Contract

### Summary

The catalog's top-level namespaces become a fixed set of business verticals declared in deployment config, with use-case domains below them. The three registered tables are moved into that layout. The web UI stops offering catalog writes, so all registration and namespace changes go through the API.

### Problem Frame

Applied's datasets serve several verticals: automotive, trucking, mining, robotics. The catalog's namespaces were chosen by copying S3 prefixes, such as `scenario_dataset_export/1stage_scenario_dataset_v2` and `world_foundation_model/datasets/silver/metadata`. Those paths describe how storage was laid out by whichever pipeline wrote the data, not who uses it. A user looking for robotics data cannot tell from the tree where to look, and nothing stops new registrations from adding more storage-derived top-level names.

The catalog itself never derived these paths. The registrant chooses the namespace at registration time (`catalog-store/src/namespace.rs`), so the layout follows only convention.

### Requirements

**Vertical layout**

- R1. The deployment declares an allowlist of vertical names; when the allowlist is set, creating a top-level namespace whose name is not on it is rejected with a client error that names the allowed verticals.
- R2. When the allowlist is set, registering a table directly in the root namespace is rejected, so every table lives under a vertical.
- R3. Namespaces below a vertical (domains and deeper) are not validated against any list.
- R4. Top-level namespaces that already exist but are not on the allowlist continue to read, list, and describe normally, and can be dropped once empty. Every create request for a non-allowlisted top-level name is rejected, including `ExistOk` and `Overwrite` on an existing one.

**Table moves**

- R5. The three registered tables move to these paths, with their S3 locations unchanged:

| Current path | New path |
|---|---|
| `scenario_dataset_export/1stage_scenario_dataset_v2/2026-11-07_06-46-14` | `automotive/scenarios/1stage_scenario_dataset_v2` |
| `world_foundation_model/datasets/silver/metadata/abc_130k` | `robotics/episodes/abc_130k` |
| `vlm/corpus/video_embeddings` | `automotive/vlm/video_embeddings` |

- R6. Each move is a deregister at the old path and a register at the new path that carries over the table's owner, description, and client properties; old paths return table-not-found afterwards.
- R7. After the moves, the now-empty storage-derived namespaces (`scenario_dataset_export`, `world_foundation_model`, `vlm`, and their descendants) are dropped.
- R8. The move is delivered as a repeatable, reviewable procedure against a catalog URL, with a dry-run that prints the planned calls; running it against the deployed catalog is an operator action.

**Read-only UI**

- R9. The web UI offers no catalog writes: the Register table, Create namespace, and Drop namespace controls are removed, along with UI copy that tells users to perform those actions.

### Acceptance Examples

- AE1. **Covers R1.** Given allowlist `automotive,trucking,mining,robotics`, creating namespace `automotive` succeeds; creating `world_foundation_model` fails with a client error listing the four verticals.
- AE2. **Covers R1, R3.** Given the same allowlist and existing `automotive`, creating `automotive/scenarios` and `automotive/scenarios/v2` succeeds.
- AE3. **Covers R2.** Given the allowlist is set, registering table `foo` at the root fails; registering `automotive/scenarios/foo` succeeds.
- AE4. **Covers R4.** Given namespace `vlm` existed before the allowlist was configured, listing and describing `vlm` and its tables still works, and dropping `vlm` after it is emptied succeeds.
- AE5. **Covers R1.** Given no allowlist is configured, creating any top-level namespace succeeds, matching current behavior.
- AE6. **Covers R6.** After the move, describing `vlm/corpus/video_embeddings` returns table-not-found. Describing `automotive/vlm/video_embeddings` returns the original S3 location, owner, description, and client properties, and `catalog.*` values are fresh server values rather than copied ones.

### Key Decisions

- **Primary vertical only, no cross-vertical tagging.** No current table serves more than one vertical, so a single-parent tree covers discovery. (session-settled: user-directed — chosen over a primary vertical plus a validated `verticals` tag: no cross-vertical tables exist today.) Governs R1, R5.
- **Second level is a use-case domain, free-form.** Examples: `scenarios`, `vlm`, `episodes`. (session-settled: user-directed — chosen over owning team or data stage: discovery is by use case.) Governs R3.
- **Enforce the top level through a deployment-config allowlist.** (session-settled: user-directed — chosen over convention-only and admin-created top levels: prevents new storage-derived top-level names.) Governs R1, R2.
- **Allowlist unset means unrestricted.** This keeps local development and tests unchanged. The deployed environment sets the allowlist explicitly. Governs R1, R2.
- **Move by deregister plus register, no RenameTable.** (session-settled: user-directed — chosen over implementing the spec `RenameTable` operation: three tables do not justify a new operation.) Re-registration resets `registered_at`. Governs R6, R8.
- **Old paths stop resolving.** (session-settled: user-directed — chosen over keeping aliases.) Governs R6.
- **UI is read-only.** (session-settled: user-directed — chosen over keeping namespace create/drop in the UI: writes stay operator-controlled through the API.) Governs R9.

### Scope Boundaries

- Cross-vertical tags, a vertical filter, and "also used by this vertical" sections are deferred until a table serves more than one vertical.
- A cross-catalog tables list page is a separate feature.
- The spec `RenameTable` operation and old-path aliases are not implemented.
- Validating domain names under a vertical is out of scope.
- UI copy changes are limited to removing write affordances; the UI does not otherwise restyle namespace pages around verticals.

### Dependencies / Assumptions

- No job or notebook outside the UI depends on the three current table identifiers; the move breaks any that do.
- The deployed catalog's Postgres holds exactly the three tables in R5; the move procedure reports and stops on any additional table under the namespaces it drops.

### Sources / Research

- Namespace creation, registration, and the reserved `catalog.*` property prefix are implemented in `catalog-store/src/namespace.rs`; config comes from environment variables in `catalog-api/src/config.rs`.
- Namespaces use a single-parent tree with `ON DELETE RESTRICT` (`catalog-store/migrations/`), so a namespace can be dropped only after its children and tables are removed.
- UI write controls are in `frontend/src/components/register-dialog.tsx`, `frontend/src/components/create-namespace-dialog.tsx`, and the drop action in `frontend/src/components/namespace-page.tsx`.
- Lance namespace object model: https://lance.org/format/namespace/object-relationship/

Product Contract preservation: Product Contract unchanged. R2 already holds today because `register_table` rejects root-level tables; the plan adds coverage only.

---

## Planning Contract

### Key Technical Decisions

- KTD1. **The allowlist lives on `PostgresNamespace`, set through a builder method.** `create_namespace` is the only path that adds a top-level namespace, so a single check there covers REST and any in-process client. A builder keeps `PostgresNamespace::new` callers (`catalog-api/src/main.rs`, `catalog-store/tests/support/mod.rs`) unchanged. Governs R1, R3, R4.
- KTD2. **`CATALOG_VERTICALS` holds a comma-separated list; unset or empty means unrestricted.** It follows the existing env-only config in `catalog-api/src/config.rs`. Names are trimmed, empty entries dropped, and each name must pass the same segment validation as a namespace name, so an invalid list fails startup. Governs R1.
- KTD3. **Reject create requests for non-allowlisted top-level names in every mode.** `existok` on a legacy top-level name is rejected too: it is a creation request for a non-vertical. Read, list, describe, exists, and drop never consult the allowlist (R4). The rejection is `InvalidInput` (HTTP 400) and lists the allowed verticals. Governs R1, R4.
- KTD4. **The move is a bash script, `scripts/move-tables.sh`, using `curl` and `jq`.** This matches the runbook's curl-based smoke test. It reads `old new` id pairs, joined with `/`, from a mapping file and defaults to a dry run that prints each call. With `--apply`, it performs these steps for each pair:
  1. Describe the old table.
  2. Create the new parent namespaces in `ExistOk` mode.
  3. Register the new table in `create` mode with the old location, owner, description, and client properties. `catalog.*` keys are stripped.
  4. Deregister the old table.
  5. Drop each old namespace upward from the table's parent until a drop returns `NamespaceNotEmpty` or the root is reached.

  Registering before deregistering means the dataset is always registered somewhere. On a rerun, a pair whose old path is gone and whose new path exists skips registration and deregistration and resumes namespace cleanup. Cleanup treats an already-dropped namespace as done, stops quietly at a non-empty one, and fails on any other error. Dry-run output prints method, path, and body only, never the token. Governs R6, R7, R8.
- KTD5. **Delete the UI write path end to end.** This includes the dialogs, the drop action, and their API helpers, response schemas, property-editing utilities, and tests that only those writes use. Shared utilities that reads still use (`segmentError` inside `joinId`) stay. Governs R9.

### Assumptions

- `jq` and `curl` are available where the operator runs the move; the runbook lists them as prerequisites.
- Deployed access uses the runbook's `Authorization: Bearer $(apps-platform auth token)` pattern; the script accepts an optional bearer token from `CATALOG_TOKEN`.

---

## Implementation Units

### U1. Top-level vertical allowlist

- **Goal:** Enforce R1 and R3 through R4 in the store, and wire the setting from config and deployment.
- **Requirements:** R1, R2, R3, R4; KTD1, KTD2, KTD3.
- **Files:** `catalog-store/src/namespace.rs`, `catalog-api/src/config.rs`, `catalog-api/src/main.rs`, `project.toml`, `README.md` (configuration table), `ARCHITECTURE.md` (request handling), `catalog-store/tests/namespace.rs`.
- **Approach:** Add an optional allowed-top-level set to `PostgresNamespace`, with a builder method to set it. In `create_namespace`, after `validate_names`, reject a one-segment path that is not in the set. Add `verticals: Vec<String>` to `AppConfig`, parsed from `CATALOG_VERTICALS` by a pure list parser with its own unit test, which avoids mutating the process environment in parallel tests. Apply it in `main.rs`. Set `CATALOG_VERTICALS = "automotive,trucking,mining,robotics"` under `[cloudrun.env_vars]`.
- **Test scenarios** (new `#[tokio::test]` in `catalog-store/tests/namespace.rs`, using a namespace built with the allowlist):
  - AE1: creating `automotive` succeeds. Creating `world_foundation_model` fails with `InvalidInput`, and the message names the allowed verticals.
  - AE2: creating `automotive/scenarios` and then `automotive/scenarios/v2` succeeds.
  - KTD3: `ExistOk` and `Overwrite` creates of a non-allowlisted top-level name are rejected.
  - AE4: a store without the allowlist creates `vlm/corpus` and registers a table. The same Postgres schema reopened with the allowlist still lists, describes, and checks the existence of `vlm` and its table, and drops `vlm` once it is emptied.
  - AE3/R2: registering a table at the root fails, while registering under `automotive/scenarios` succeeds.
  - AE5: existing lifecycle tests, which use no allowlist, still pass unchanged.
  - Config: `CATALOG_VERTICALS` unset or empty gives an empty list, and `" automotive, ,mining "` gives `["automotive", "mining"]`. A name containing `$` is an error.
- **Verification:** `cargo test --workspace` with `CATALOG_TEST_DATABASE_URL` set.

### U2. Read-only web UI

- **Goal:** Remove every catalog write control from the UI.
- **Requirements:** R9; KTD5.
- **Files:** `frontend/src/components/namespace-page.tsx`. Delete `frontend/src/components/register-dialog.tsx`, `frontend/src/components/create-namespace-dialog.tsx`, and whichever of `frontend/src/components/properties-editor.tsx`, `frontend/src/components/confirm-dialog.tsx`, `frontend/src/lib/properties.ts`, and `frontend/src/lib/properties.test.ts` become unused. Remove the write helpers and response schemas from `frontend/src/lib/api.ts`, `frontend/src/lib/schemas.ts`, `frontend/src/lib/api.test.ts`, and `frontend/src/lib/schemas.test.ts`. Update `docs/FRONTEND.md`.
- **Approach:** Remove the Create, Register, and Drop buttons and their state and handlers from `namespace-page.tsx`, along with any "through the API" toast copy. Remove each export only after confirming it has no remaining importer. `api.test.ts` uses `dropNamespaceResponseSchema` only as a sample empty-body schema; switch that assertion to a remaining read schema that accepts `{}` rather than deleting the decode coverage.
- **Test scenarios:** `bun run typecheck`, `bun run test`, and `bun run build` pass after the deletions. The namespace page renders without write buttons at the root and at a nested namespace, checked in a browser against a local catalog.
- **Verification:** The frontend commands above, plus a browser check of `/namespaces` and a nested namespace page.

### U3. Table move script and runbook

- **Goal:** Provide the reviewable, repeatable procedure that moves the three tables into the vertical layout.
- **Requirements:** R5, R6, R7, R8; KTD4.
- **Files:** `scripts/move-tables.sh` (new), `docs/RUNBOOK-DEPLOY.md` (prerequisites for `curl` and `jq`, a section with the R5 mapping and the dry-run then `--apply` steps, and a smoke test that creates its namespace under a vertical).
- **Approach:** Implement the steps in KTD4. Require `CATALOG_URL`, and accept an optional `CATALOG_TOKEN`. Encode `/`-separated ids into `$`-joined path segments, and print every call in dry-run mode. Under `--apply`, exit non-zero on the first unexpected error. When the new table already exists with the same location, skip registration so a rerun resumes. Report and stop at a non-empty old namespace instead of failing.
- **Test scenarios** (run locally against `catalog-api` on the test Postgres database, with three local Lance datasets registered at the R5 old paths plus one extra table under `vlm/other`):
  - A dry run prints the planned calls and changes nothing.
  - `--apply` makes the new paths describe to the original locations, owners, descriptions, and client properties, while the old paths return 404. `scenario_dataset_export` and `world_foundation_model` are dropped. `vlm` stays because `vlm/other` remains, and the script reports that.
  - A second `--apply` succeeds and resumes cleanup without re-registering.
  - A dry run with a sentinel `CATALOG_TOKEN` never prints the sentinel.
- **Verification:** The local run above. `bash -n scripts/move-tables.sh` passes, and so does `shellcheck` when it is installed.

---

## Verification Contract

| Check | Command | Covers |
|---|---|---|
| Rust build and tests | `CATALOG_TEST_DATABASE_URL=… cargo test --workspace` | U1 |
| Rust lint | `cargo clippy --workspace --all-targets -- -D warnings` | U1 |
| Frontend | `cd frontend && bun run typecheck && bun run test && bun run build` | U2 |
| UI surface | Browser check of the namespace pages against a local catalog | U2 |
| Move procedure | Local dry run, then two `--apply` runs per the U3 scenarios | U3 |

---

## Definition of Done

- All units are done, and every check in the Verification Contract passes.
- `README.md`, `ARCHITECTURE.md`, `docs/FRONTEND.md`, and `docs/RUNBOOK-DEPLOY.md` describe the allowlist, the read-only UI, and the move procedure.
- The diff contains no dead UI write code, abandoned experiments, or leftover local test fixtures.
- The move has not been run against any deployed catalog; that remains an operator step.
