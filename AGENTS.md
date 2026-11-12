# AGENTS.md

This repo is a Lance REST catalog service in Rust (axum, sqlx, the `lance` and
`lance-namespace` crates) with a React UI, deployed as a single container on Apps
Platform (Cloud Run).

- Routes, configuration, local setup: `README.md`.
- Crates, data model, current version and caching: `ARCHITECTURE.md`.
- UI behavior and frontend commands: `docs/FRONTEND.md`.
- `docs/design.md` is a superseded historical design.
- Current plan: `docs/plans/2026-11-11-1747-feat-version-pinned-cache-plan.md`.

## Conventions

- Rust edition 2021, toolchain pinned in `rust-toolchain.toml`.
- Before committing, these must pass (CI runs them): `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, and
  `cargo test --workspace --all-targets`. Postgres-backed tests need
  `CATALOG_TEST_DATABASE_URL`; without it they skip, and CI sets
  `CATALOG_REQUIRE_DB_TESTS=1` to forbid skipping. Frontend changes must pass
  `bun run build` in `frontend/` (CI).
- LF line endings, UTF-8, 4-space indentation for Rust and 2 for YAML, TOML, and
  JSON (see `.editorconfig`).
- Schema changes are new files in `catalog-store/migrations/`; never edit an
  applied migration.

## Commit messages

PRs are squash-merged, so the PR title becomes the commit message and must follow
[Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>)?: <description>
```

Types: `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor`,
`revert`, `style`, `test`. Example: `fix(api): return 404 for unknown table id`.

CI (`.github/workflows/commit-lint.yml`) checks only the PR title. Commit subjects
should follow the same format; enable the local check with:

```
git config core.hooksPath .githooks
```
