#!/usr/bin/env bash
# Moves registered tables to new catalog paths without touching their data.
#
# Usage: CATALOG_URL=https://... [CATALOG_TOKEN=...] scripts/move-tables.sh MAPPING [--apply]
#
# MAPPING has one "old/table/path new/table/path" pair per line; blank lines and
# lines starting with '#' are ignored. Without --apply, prints the planned calls.
#
# Per pair: create the new parent namespaces, register the new path with the old
# location, owner, description, and client properties, deregister the old path,
# then drop the old parent namespaces upward until one still has content. The
# new path is registered before the old one is deregistered, so a rerun after a
# failure resumes. Re-registration resets catalog.registered_at.
set -euo pipefail

usage() {
    echo "usage: CATALOG_URL=... [CATALOG_TOKEN=...] $0 MAPPING [--apply]" >&2
    exit 2
}

[[ $# -ge 1 && $# -le 2 ]] || usage
mapping=$1
apply=false
if [[ $# -eq 2 ]]; then
    [[ $2 == --apply ]] || usage
    apply=true
fi
[[ -r $mapping ]] || { echo "cannot read $mapping" >&2; exit 2; }
: "${CATALOG_URL:?CATALOG_URL must be set}"
command -v jq >/dev/null || { echo "jq is required" >&2; exit 2; }

# Spec error codes (lance-namespace ErrorCode).
NAMESPACE_NOT_FOUND=1
NAMESPACE_NOT_EMPTY=3

# Path-segment form of a slash-separated id: segments joined by '$'.
rest_id() { tr '/' '$' <<<"$1"; }

# call METHOD PATH BODY: sets $status and $body. Does not exit on HTTP errors.
call() {
    local out
    out=$(mktemp)
    local args=(-sS -X "$1" -o "$out" -w '%{http_code}' -H 'content-type: application/json')
    [[ -n ${CATALOG_TOKEN:-} ]] && args+=(-H "Authorization: Bearer $CATALOG_TOKEN")
    status=$(curl "${args[@]}" --data "$3" "$CATALOG_URL$2")
    body=$(cat "$out")
    rm -f "$out"
}

error_code() { jq -r '.code // empty' <<<"$body" 2>/dev/null; }

fail() {
    echo "error: $1 (HTTP $status): $body" >&2
    exit 1
}

# write METHOD PATH BODY: prints the call, and performs it only under --apply.
write() {
    echo "  $1 $2 $3"
    if $apply; then
        call "$@"
    else
        status=dry-run
    fi
}

describe_table() {
    call POST "/v1/table/$(rest_id "$1")/describe" '{}'
}

move() {
    local old=$1 new=$2
    echo "move $old -> $new"

    describe_table "$new"
    local new_status=$status new_body=$body
    describe_table "$old"
    if [[ $status == 404 ]]; then
        [[ $new_status == 200 ]] || fail "$old is not registered and $new does not exist"
        echo "  $old already deregistered"
        drop_empty_parents "$old"
        return
    fi
    [[ $status == 200 ]] || fail "describe $old"
    local location properties
    location=$(jq -r '.location // .table_uri' <<<"$body")
    # catalog.* keys are server-managed and rejected on register.
    properties=$(jq -c '.properties // {} | with_entries(select(.key | startswith("catalog.") | not))' <<<"$body")

    if [[ $new_status == 200 ]]; then
        [[ $(jq -r '.location // .table_uri' <<<"$new_body") == "$location" ]] ||
            { status=$new_status body=$new_body; fail "$new exists with a different location"; }
        echo "  $new already registered at $location"
    else
        local parent=${new%/*} prefix=""
        IFS=/ read -ra segments <<<"$parent"
        for segment in "${segments[@]}"; do
            prefix=${prefix:+$prefix/}$segment
            write POST "/v1/namespace/$(rest_id "$prefix")/create" '{"mode":"ExistOk"}'
            [[ $status == 200 || $status == dry-run ]] || fail "create namespace $prefix"
        done
        write POST "/v1/table/$(rest_id "$new")/register" \
            "$(jq -cn --arg location "$location" --argjson properties "$properties" \
                '{location: $location, properties: $properties}')"
        [[ $status == 200 || $status == dry-run ]] || fail "register $new"
    fi

    write POST "/v1/table/$(rest_id "$old")/deregister" '{}'
    [[ $status == 200 || $status == dry-run ]] || fail "deregister $old"

    drop_empty_parents "$old"
}

# Drops the namespaces above a table id, deepest first, stopping at one that still has content.
drop_empty_parents() {
    $apply || { echo "  then drop each now-empty namespace above $1"; return; }
    local ns=${1%/*}
    while [[ -n $ns ]]; do
        write POST "/v1/namespace/$(rest_id "$ns")/drop" '{}'
        if [[ $status == 409 && $(error_code) == "$NAMESPACE_NOT_EMPTY" ]]; then
            echo "  kept $ns: it still has child namespaces or tables"
            return
        fi
        [[ $status == 200 || ($status == 404 && $(error_code) == "$NAMESPACE_NOT_FOUND") ]] ||
            fail "drop namespace $ns"
        if [[ $ns == */* ]]; then ns=${ns%/*}; else ns=""; fi
    done
}

$apply || echo "dry run: no changes; rerun with --apply to perform these calls"
while read -r old new extra || [[ -n ${old:-} ]]; do
    [[ -z ${old:-} || $old == \#* ]] && continue
    [[ -n ${new:-} && -z ${extra:-} ]] || { echo "bad mapping line: $old $new $extra" >&2; exit 2; }
    move "$old" "$new"
done <"$mapping"
