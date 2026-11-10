#!/bin/sh
# On Cloud Run with `enable_postgres = true`, Apps Platform sets INSTANCE_CONNECTION_NAME
# (`/cloudsql/<project:region:instance>`), DB_USER (IAM principal), and DB_NAME. Run the Cloud SQL
# Auth Proxy with IAM auth over private IP and point the catalog at it. Without it, DATABASE_URL
# must already be set.
set -eu

if [ -n "${INSTANCE_CONNECTION_NAME:-}" ]; then
    instance="${INSTANCE_CONNECTION_NAME#/cloudsql/}"
    cloud-sql-proxy --auto-iam-authn --private-ip --port 5432 "$instance" &
    # The IAM user contains `@`, which must be percent-encoded in the URL userinfo.
    user=$(printf '%s' "$DB_USER" | sed 's/@/%40/g')
    export DATABASE_URL="postgres://${user}@127.0.0.1:5432/${DB_NAME}?sslmode=disable"
    # Apps Platform names the service schema after the service, hyphens to underscores.
    export CATALOG_DB_SCHEMA="${CATALOG_DB_SCHEMA:-$(printf '%s' "${K_SERVICE:-lance-catalog}" | tr '-' '_')}"
fi

exec /usr/local/bin/catalog-api "$@"
