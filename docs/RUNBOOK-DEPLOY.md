# Deployment runbook

How to deploy `catalog-api` to Cloud Run via Applied's Apps Platform. The app is
configured by [`project.toml`](../project.toml) at the repo root and built from
the repo-root [`Dockerfile`](../Dockerfile) (frontend build stage + static musl
Rust binary + bundled SPA).

## Prerequisites

- The `apps-platform` CLI, authenticated (`apps-platform auth login`).
- Docker (the Rust container is built with `--local`).
- AWS credentials (access key id + secret) with `list` + `get` + `delete` on
  every bucket declared in [`catalog-config.yaml`](../catalog-config.yaml) and
  `get`/`put` on the `_catalog/` prefixes (the overlay needs conditional
  `PutMode::Create`/`Update`, i.e. `If-None-Match`/`If-Match`).

## 1. Pick an environment

```sh
apps-platform app environment list
apps-platform app environment use experimental-staging
```

> The target buckets hold customer-derived perception data (vehicle data,
> simulation results) and the TTL engine hard-deletes it — the platform's
> sensitive-data policy flags this category as needing extra care on
> `experimental`/`experimental-staging`. The `anaheim` and `internal`
> environments are isolated from platform-team access for exactly this case;
> request access before pointing `catalog-config.yaml` at a real bucket outside
> local/scratch testing.

## 2. Review `catalog-config.yaml`

[`catalog-config.yaml`](../catalog-config.yaml) (repo root, committed, baked
into the image) declares this deployment's region, target buckets, each
bucket's registered namespace prefixes, and the admin email list. The service
is regional: it must run in the same region as the buckets it declares.
Changing scope (a new bucket, a new namespace) is a PR + redeploy, not a
runtime config flip — review it before every deploy to a new environment, and
confirm the region matches the target Cloud Run environment.

## 3. Review `project.toml`

[`project.toml`](../project.toml) sets the service name (`lance-catalog`),
`enable_secrets = true`, Cloud Run resources, and the non-secret runtime config
(`[cloudrun.env_vars]`: registry/audit/storage-scan/users/meta URIs, AWS
region, `RUST_LOG`). By default any Applied FTE can reach the app (IAP +
Trident); restrict with `allowed_usergroups` if needed. The active environment
profile's own `[cloudrun]`/env var settings, if any, take precedence over
`project.toml`'s — check `apps-platform app environment show <name>` if a
deploy doesn't seem to pick up a value set here.

The overlay (`CATALOG_META_BASE_URI`) MUST be an `s3://` URI — it relies on S3
conditional writes, which `LocalFileSystem` does not implement. `memory` is only
for local runs.

## 4. Set the AWS credentials as secrets

Secrets are read from Secret Manager at startup (the platform does not inject
them as env vars). Set them under the plain key names the app expects; the
platform prefixes them with the service name automatically:

```sh
apps-platform app secret set AWS_ACCESS_KEY_ID <access-key-id>
apps-platform app secret set AWS_SECRET_ACCESS_KEY <secret-access-key>
# Optional, only if using temporary credentials:
# apps-platform app secret set AWS_SESSION_TOKEN <session-token>

apps-platform app secret list   # verify: lance-catalog-aws-access-key-id, ...
```

At startup the app fetches these via the metadata server + Secret Manager REST
and passes them to the S3 client. If `AWS_ACCESS_KEY_ID` is already in the
environment (local dev), the fetch is skipped.

## 5. Deploy

```sh
apps-platform app deploy --local   # builds the Dockerfile locally, deploys to Cloud Run
```

The CLI prints the service URL on success. The DNS suffix doesn't always
match the environment name verbatim — e.g. `experimental-staging` serves at
`.experimental.staging.apps.applied.dev` but `experimental-prod` is just
`.experimental.apps.applied.dev` (run `apps-platform docs get environments`
for the full per-environment suffix table). On boot the app binds `$PORT`,
hydrates AWS credentials, and serves both the API and the SPA. `/readyz`
returns `503` until the first view load, then `200`.

## 6. Schedule the sync

There is no background sync loop (Cloud Run throttles CPU between requests).
Cloud Scheduler drives it by calling the sync endpoint on a cron:

```sh
apps-platform app schedule create sync \
  --endpoint /internal/jobs/sync \
  --cron "*/30 * * * *"      # every 30 minutes

apps-platform app schedule list
```

The Scheduler authenticates with the app's platform identity (its service
account bypasses the `allowed_usergroups` check), so the endpoint needs no
app-level auth. It is idempotent under Scheduler retries and double-fires: the
snapshot is written last-wins, and same-instance runs are serialized in-process.

If an environment still has a `sweep` schedule from a prior deploy (targeting
the retired `/internal/jobs/sweep` endpoint), delete it — it now 404s and
should not keep firing:

```sh
apps-platform app schedule delete sweep
```

## 7. Smoke test

Through the browser (IAP handles login) or with an ID token
(`apps-platform auth token`):

```sh
URL=<the URL printed by `apps-platform app deploy` in step 5>
TOKEN=$(apps-platform auth token)
auth() { curl -s -H "Authorization: Bearer $TOKEN" "$@"; }

auth "$URL/readyz" -o /dev/null -w '%{http_code}\n'         # 200 once loaded
# Register a table FIRST — the sync only processes registered tables. <id> is
# the composite <region>:<bucket>:<namespace>:<name> — the (bucket, namespace)
# must be declared in catalog-config.yaml.
auth -X PUT "$URL/v1/table/<id>" -H 'content-type: application/json' \
  -d '{"owner":"you@applied.co","ttl_policy":{"keep_last_n":10,"max_age_days":90}}'
auth -X POST "$URL/internal/jobs/sync" | python3 -m json.tool   # {tables_checked,...}
auth "$URL/v1/tables" | python3 -m json.tool                # registered tables
auth "$URL/ext/v1/tables/<id>/ttl/dryrun" | python3 -m json.tool   # review before any apply
```

Open `$URL/` in a browser for the SPA. Tail logs with:

```sh
apps-platform app tail
```

For a first TTL apply, run it against a scratch namespace (a throwaway bucket
and namespace declared in `catalog-config.yaml`) before pointing at real
data — the delete is irreversible. See [`RUNBOOK-TTL.md`](RUNBOOK-TTL.md).

## Networking note (static egress IP)

If the S3 bucket policy restricts by source IP, set
`use_static_egress_ip = true` in `[cloudrun]` and allowlist the environment's
static egress IP (listed in the platform's project-toml docs) on the AWS side.
Otherwise leave it `false` — outbound S3 traffic goes over the internet.

## Rollback / teardown

```sh
apps-platform app deploy --local        # redeploy a prior commit to roll back
apps-platform app schedule delete sync # stop the sync
apps-platform app delete                # tear the service down
```

## Frontend

The SPA is built into the container (the Dockerfile's `frontend` stage runs
`bun run build`) and served same-origin at `/`. There is no separate frontend
host or deploy step. For local hot-reload development see
[`frontend/docs/DEPLOY.md`](../frontend/docs/DEPLOY.md).
