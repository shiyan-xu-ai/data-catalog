# Deployment runbook

How to deploy `catalog-api` to Cloud Run via Applied's Apps Platform. The app is
configured by [`project.toml`](../project.toml) and built from the repo-root
[`Dockerfile`](../Dockerfile) (frontend build, Rust binary, bundled SPA, Cloud SQL
Auth Proxy).

## Prerequisites

- The `apps-platform` CLI, authenticated (`apps-platform auth login`).
- Docker (the container is built with `--local`).
- AWS credentials with `list` + `get` on every bucket holding datasets you will
  register. The catalog only reads datasets; it never writes or deletes them.

## 1. Pick an environment

```sh
apps-platform app environment list
apps-platform app environment use experimental-staging
```

Registered datasets may hold customer-derived data. Use an environment approved
for that data and keep the platform's IAP access boundary; the application does
no authentication of its own.

## 2. Postgres

`project.toml` sets `enable_postgres = true`, so the platform provisions a schema
named after the service (`lance_catalog`) on a shared Cloud SQL instance and sets
`INSTANCE_CONNECTION_NAME`, `DB_USER`, and `DB_NAME`. On start,
[`docker-entrypoint.sh`](../docker-entrypoint.sh) runs `cloud-sql-proxy
--auto-iam-authn --private-ip`, derives `DATABASE_URL` and `CATALOG_DB_SCHEMA`,
and starts the server, which applies migrations. `/readyz` returns `503` until the
database answers.

Inspect the database locally with `apps-platform app connect-db --connect`.

## 3. Set the AWS credentials as secrets

```sh
apps-platform app secret set AWS_ACCESS_KEY_ID <access-key-id>
apps-platform app secret set AWS_SECRET_ACCESS_KEY <secret-access-key>
# Optional, temporary credentials only:
# apps-platform app secret set AWS_SESSION_TOKEN <session-token>
```

The app reads them from Secret Manager at startup unless `AWS_ACCESS_KEY_ID` is
already in the environment, and uses them to open registered datasets.

## 4. Deploy

```sh
apps-platform app deploy --local
```

Environments that still have a `sync` Cloud Scheduler job from the previous
catalog should delete it; the endpoint no longer exists:

```sh
apps-platform app schedule delete sync
```

## 5. Smoke test

```sh
URL=<service URL printed by deploy>
TOKEN=$(apps-platform auth token)
auth() { curl -s -H "Authorization: Bearer $TOKEN" "$@"; }

auth "$URL/readyz" -o /dev/null -w '%{http_code}\n'               # 200
auth -X POST "$URL/v1/namespace/prod/create" -H 'content-type: application/json' -d '{}'
auth -X POST "$URL/v1/table/prod\$users/register" -H 'content-type: application/json' \
  -d '{"location":"s3://bucket/path/users.lance","properties":{"owner":"you@applied.co"}}'
auth -X POST "$URL/v1/table/prod\$users/describe?load_detailed_metadata=true" \
  -H 'content-type: application/json' -d '{}' | python3 -m json.tool
```

Open `$URL/` for the UI; tail logs with `apps-platform app tail`.

Data from the previous S3-only catalog (`_catalog/` registry and registration
JSON) is neither read nor deleted; re-register the datasets you need.

## Networking note (static egress IP)

If a bucket policy restricts by source IP, set `use_static_egress_ip = true` in
`[cloudrun]` and allowlist the environment's static egress IP on the AWS side.

## Rollback / teardown

```sh
apps-platform app deploy --local   # redeploy a prior commit
apps-platform app delete           # tear the service down
```
