# AGENTS.md

This repo is a Lance REST catalog service: Postgres stores namespaces, table
registrations, and each table's current version; Lance manifests stay authoritative
for each version's contents. Written in
Rust (axum, sqlx, the `lance` and `lance-namespace` crates), deployed as a single
container on Apps Platform (Cloud Run).

- Architecture: `ARCHITECTURE.md`. `docs/design.md` is a superseded historical design.
- Current plan: `docs/plans/2026-11-11-1747-feat-version-pinned-cache-plan.md`.

## Workspace layout

- `catalog-store` — `PostgresNamespace` (`LanceNamespace` impl) and sqlx
  migrations in `catalog-store/migrations/`.
- `catalog-api` — the `catalog-api` binary: config, spec router, UI serving.

## Conventions

- Rust edition 2021, toolchain pinned in `rust-toolchain.toml`.
- `cargo build --workspace` and `cargo test --workspace` must pass before
  committing. Postgres-backed tests need `CATALOG_TEST_DATABASE_URL`; without it
  they skip, and CI sets `CATALOG_REQUIRE_DB_TESTS=1` to forbid skipping.
- 4-space indentation, LF line endings, UTF-8 (see `.editorconfig`).
- Schema changes are new files in `catalog-store/migrations/`; never edit an
  applied migration.

## Commit messages

PR titles must follow [Conventional Commits](https://www.conventionalcommits.org/)
(PRs are squash-merged, so the PR title becomes the final commit message):

```
<type>(<scope>)?: <description>
```

Types: `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor`,
`revert`, `style`, `test`. Example: `fix(api): return 404 for unknown table id`.

This is enforced in CI (`.github/workflows/commit-lint.yml`) against the PR
title only — individual commits on a branch are free-form and get squashed
away on merge. Commit subjects are still encouraged to follow the same format
(it makes for a clean history pre-squash); enable a local pre-commit check
with:

```
git config core.hooksPath .githooks
```
