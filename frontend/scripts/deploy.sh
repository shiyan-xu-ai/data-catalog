#!/usr/bin/env bash
# Deploy the data-catalog frontend static build to S3 + CloudFront.
#
# Usage:
#   ./frontend/scripts/deploy.sh <bucket-name> <distribution-id>
#
# Prerequisites:
#   - aws CLI v2 configured with credentials that can write to <bucket-name>
#     and invalidate <distribution-id>.
#   - `bun install && bun run build` (or `npm install && npm run build`) has been
#     run in frontend/ so frontend/dist/ exists.
#
# This script does a `aws s3 sync` of dist/ into the bucket root, then issues a
# CloudFront invalidation so the new build is visible immediately. See
# frontend/docs/DEPLOY.md for the one-time S3 static-website + CloudFront setup.

set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <bucket-name> <distribution-id>" >&2
  exit 1
fi

BUCKET="$1"
DISTRIBUTION_ID="$2"
DIST_DIR="$(cd "$(dirname "$0")/.." && pwd)/dist"

if [ ! -d "$DIST_DIR" ]; then
  echo "error: $DIST_DIR does not exist. Run 'bun install && bun run build' in frontend/ first." >&2
  exit 1
fi

echo ">> syncing $DIST_DIR to s3://$BUCKET/"
aws s3 sync "$DIST_DIR" "s3://$BUCKET/" \
  --delete \
  --cache-control "public, max-age=300" \
  --exclude ".*"

echo ">> invalidating CloudFront distribution $DISTRIBUTION_ID"
aws cloudfront create-invalidation \
  --distribution-id "$DISTRIBUTION_ID" \
  --paths "/*"

echo ">> done."
