# Deployment runbook

Deploys `catalog-api` to Cloud Run through Applied's Apps Platform. The app is
configured by [`project.toml`](../project.toml) and built from the
[`Dockerfile`](../Dockerfile).

## Prerequisites

- The `apps-platform` CLI, logged in (`apps-platform auth login`).
- Docker; the image builds locally (`--local`).
- Storage credentials with list and read access on every bucket that holds
  datasets you will register or media you will play. The catalog never writes
  or deletes data.
- `curl` and `jq`.

## 1. Pick an environment

```sh
apps-platform app environment list
apps-platform app environment use experimental-staging
```

Registered datasets may hold customer-derived data. Use an environment approved
for that data and keep the platform's IAP in front; the app has no
authentication of its own.

## 2. Postgres

`enable_postgres = true` in `project.toml` makes the platform provision a schema
named after the service (`lance_catalog`) on a shared Cloud SQL instance and set
`INSTANCE_CONNECTION_NAME`, `DB_USER`, and `DB_NAME`.
[`docker-entrypoint.sh`](../docker-entrypoint.sh) starts the Cloud SQL Auth
Proxy, derives `DATABASE_URL` and `CATALOG_DB_SCHEMA`, and starts the server,
which applies migrations. `/readyz` returns 503 until the database answers.

Inspect the database with `apps-platform app connect-db --connect`.

## 3. Storage credentials

```sh
apps-platform app secret set AWS_ACCESS_KEY_ID <access-key-id>
apps-platform app secret set AWS_SECRET_ACCESS_KEY <secret-access-key>
# Temporary credentials only:
# apps-platform app secret set AWS_SESSION_TOKEN <session-token>
```

The app reads them from Secret Manager at startup unless `AWS_ACCESS_KEY_ID` is
already set. For OCI Object Storage, also set `AWS_ENDPOINT_URL` and
`AWS_REGION` (for example `us-phoenix-1`) under `[cloudrun.env_vars]` in
`project.toml`; the keys are an OCI Customer Secret Key.

Presigned media URLs carry the same credentials' read access, so scope them to
what users may read. Each media bucket's CORS rules must allow the UI's origin
and the `Range` header.

## 4. Deploy

```sh
apps-platform app deploy --local
```

## 5. Smoke test

```sh
URL=<service URL printed by deploy>
TOKEN=$(apps-platform auth token)
auth() { curl -s -H "Authorization: Bearer $TOKEN" "$@"; }
json='content-type: application/json'

auth "$URL/readyz" -o /dev/null -w '%{http_code}\n'      # 200
auth -X POST "$URL/v1/namespace/automotive/create" -H "$json" -d '{"mode":"ExistOk"}'
auth -X POST "$URL/v1/namespace/automotive\$smoke/create" -H "$json" -d '{}'
auth -X POST "$URL/v1/table/automotive\$smoke\$users/register" -H "$json" \
  -d '{"location":"s3://bucket/path/users.lance","properties":{"technical_owner":"you@applied.co"}}'
auth -X POST "$URL/v1/table/automotive\$smoke\$users/describe?load_detailed_metadata=true" \
  -H "$json" -d '{}' | jq .
# Clean up:
auth -X POST "$URL/v1/table/automotive\$smoke\$users/deregister" -H "$json" -d '{}'
auth -X POST "$URL/v1/namespace/automotive\$smoke/drop" -H "$json" -d '{}'
```

Open `$URL/` for the UI; tail logs with `apps-platform app tail`.

## Namespace layout and moving tables

Tables live under `<vertical>/<domain>/<table>`. `CATALOG_VERTICALS` lists the
allowed top-level namespaces; creating any other returns 400.

[`scripts/move-tables.sh`](../scripts/move-tables.sh) moves registered tables to
new paths without touching their data. It keeps location and properties and
drops old namespaces once empty. A move resets `catalog.registered_at`, and old
paths stop resolving.

```sh
cat > /tmp/moves.txt <<'EOF'
robotics/episodes/abc_130k robotics/silver/episodes/abc_130k
EOF
export CATALOG_URL=$URL CATALOG_TOKEN=$(apps-platform auth token)
scripts/move-tables.sh /tmp/moves.txt            # dry run: prints the calls
scripts/move-tables.sh /tmp/moves.txt --apply    # safe to rerun
```

## Static egress IP

If a bucket policy allows only certain source IPs, set
`use_static_egress_ip = true` under `[cloudrun]` and allowlist the
environment's egress IP.

## Rollback and teardown

```sh
apps-platform app deploy --local   # from a prior commit
apps-platform app delete           # removes the service
```
