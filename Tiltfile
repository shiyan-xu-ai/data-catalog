# Tiltfile — local dev on kind (or k3d).
#
# Prerequisites:
#   1. kind create cluster --name catalog-dev
#   2. tilt up          (from repo root)
#
# What this brings up:
#   - catalog-api (1 replica, real leader election via kube Lease, debug logging)
#   - MinIO in-cluster S3 compatible store with a fixture-init Job
#   - Prometheus (standalone, no operator required)
#   - Grafana (datasource + 3 dashboards pre-loaded)
#
# Port-forwards available after `tilt up`:
#   localhost:8080  → catalog-api REST API
#   localhost:9090  → catalog-api internal metrics + /healthz
#   localhost:9091  → Prometheus
#   localhost:3000  → Grafana (admin/admin)
#   localhost:9000  → MinIO S3 API
#   localhost:9001  → MinIO web console

# Scope Tilt to the kind cluster only — prevent accidental deploys to staging/prod.
allow_k8s_contexts(['kind-catalog-dev'])

# ── catalog-api image ────────────────────────────────────────────────────────────
# docker_build builds the Dockerfile at repo root and syncs to the cluster.
# The full multi-stage build runs on first `tilt up`; subsequent changes to Rust
# source trigger a rebuild. For faster iteration, consider cross-compiling the
# binary locally with `cargo build` and using live_update to sync only the binary —
# see comments in docs/dev-loop.md (not yet written; deferred to Phase 10).
docker_build(
    'catalog-api',
    context='.',
    dockerfile='Dockerfile',
    # Only rebuild when Rust source or workspace manifests change.
    only=[
        'Cargo.toml',
        'Cargo.lock',
        'rust-toolchain.toml',
        'catalog-core/',
        'catalog-store/',
        'catalog-api/',
    ],
)

# ── k8s manifests ───────────────────────────────────────────────────────────────
# Apply the local overlay (catalog-api + MinIO + fixture Job + monitoring stack).
k8s_yaml(kustomize('./deploy/overlays/local'))

# ── resource grouping and port-forwards ─────────────────────────────────────────
k8s_resource(
    'catalog-api',
    port_forwards=[
        '8080:8080',  # REST API
        '9090:9090',  # metrics + healthz
    ],
    resource_deps=['minio', 'catalog-fixture-init'],
)

k8s_resource(
    'minio',
    port_forwards=[
        '9000:9000',  # S3 API
        '9001:9001',  # web console
    ],
)

k8s_resource(
    'catalog-fixture-init',
    resource_deps=['minio'],
)

k8s_resource(
    'prometheus',
    port_forwards=['9091:9090'],
)

k8s_resource(
    'grafana',
    port_forwards=['3000:3000'],
)
