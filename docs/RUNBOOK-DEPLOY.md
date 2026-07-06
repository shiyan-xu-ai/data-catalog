# Deployment runbook

How to deploy `catalog-api` to Cloud Run via Applied's Apps Platform. The app is
configured by [`project.toml`](../project.toml) at the repo root and built from
the repo-root [`Dockerfile`](../Dockerfile) (frontend build stage + static musl
Rust binary + bundled SPA).

## Prerequisites

- The `apps-platform` CLI, authenticated (`apps-platform auth login`).
- Docker (the Rust container is built with `--local`).
- AWS credentials (access key id + secret) with `list` + `get` + `delete` on the
  sweep bucket and `get`/`put` on the `_catalog/` prefixes (the overlay needs
  conditional `PutMode::Create`/`Update`, i.e. `If-None-Match`/`If-Match`).

## 1. Pick an environment

```sh
apps-platform app environment list
apps-platform app environment use experimental-staging
```

> The sweep root holds customer-derived perception data (vehicle data,
> simulation results) and the TTL engine hard-deletes it — the platform's
> sensitive-data policy flags this category as needing extra care on
> `experimental`/`experimental-staging`. The `anaheim` and `internal`
> environments are isolated from platform-team access for exactly this case;
> request access before pointing the sweep root at a real bucket outside
> local/scratch testing.

## 2. Review `project.toml`

[`project.toml`](../project.toml) sets the service name (`lance-catalog`),
`enable_secrets = true`, Cloud Run resources, and the non-secret runtime config
(`[cloudrun.env_vars]`: sweep root, registry/audit/meta URIs, AWS region,
`RUST_LOG`). Adjust the bucket names and region for the target environment. By
default any Applied FTE can reach the app (IAP + Trident); restrict with
`allowed_usergroups` if needed. The active environment profile's own
`[cloudrun]`/env var settings, if any, take precedence over `project.toml`'s —
check `apps-platform app environment show <name>` if a deploy doesn't seem to
pick up a value set here.

The overlay (`CATALOG_META_BASE_URI`) MUST be an `s3://` URI — it relies on S3
conditional writes, which `LocalFileSystem` does not implement. `memory` is only
for local runs.

## 3. Set the AWS credentials as secrets

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

## 4. Deploy

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

## 5. Schedule the sweep

There is no background sweep loop (Cloud Run throttles CPU between requests).
Cloud Scheduler drives it by calling the sweep endpoint on a cron:

```sh
apps-platform app schedule create sweep \
  --endpoint /internal/jobs/sweep \
  --cron "*/30 * * * *"      # every 30 minutes

apps-platform app schedule list
```

The Scheduler authenticates with the app's platform identity (its service
account bypasses the `allowed_usergroups` check), so the endpoint needs no
app-level auth. It is idempotent under Scheduler retries and double-fires: the
snapshot is written last-wins, and same-instance runs are serialized in-process.

## 6. Smoke test

Through the browser (IAP handles login) or with an ID token
(`apps-platform auth token`):

```sh
URL=<the URL printed by `apps-platform app deploy` in step 4>
TOKEN=$(apps-platform auth token)
auth() { curl -s -H "Authorization: Bearer $TOKEN" "$@"; }

auth "$URL/readyz" -o /dev/null -w '%{http_code}\n'         # 200 once loaded
# Register a table FIRST — the sweep only processes registered tables. <id> is a
# table directory name directly under the sweep root.
auth -X PUT "$URL/v1/table/<id>" -H 'content-type: application/json' \
  -d '{"owner":"you@applied.co","ttl_policy":{"keep_last_n":10,"max_age_days":90}}'
auth -X POST "$URL/internal/jobs/sweep" | python3 -m json.tool   # {tables_checked,...}
auth "$URL/v1/tables" | python3 -m json.tool                # registered tables
auth "$URL/ext/v1/tables/<id>/ttl/dryrun" | python3 -m json.tool   # review before any apply
```

Open `$URL/` in a browser for the SPA. Tail logs with:

```sh
apps-platform app tail
```

For a first TTL apply, run it against a scratch S3 prefix (a throwaway
`CATALOG_SWEEP_ROOT_URI`) before pointing at real data — the delete is
irreversible. See [`RUNBOOK-TTL.md`](RUNBOOK-TTL.md).

## Networking note (static egress IP)

If the S3 bucket policy restricts by source IP, set
`use_static_egress_ip = true` in `[cloudrun]` and allowlist the environment's
static egress IP (listed in the platform's project-toml docs) on the AWS side.
Otherwise leave it `false` — outbound S3 traffic goes over the internet.

## Rollback / teardown

```sh
apps-platform app deploy --local        # redeploy a prior commit to roll back
apps-platform app schedule delete sweep # stop the sweep
apps-platform app delete                # tear the service down
```

## Frontend

The SPA is built into the container (the Dockerfile's `frontend` stage runs
`bun run build`) and served same-origin at `/`. There is no separate frontend
host or deploy step. For local hot-reload development see
[`frontend/docs/DEPLOY.md`](../frontend/docs/DEPLOY.md).
