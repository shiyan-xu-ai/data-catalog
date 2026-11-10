-- The root namespace is implicit and has no row; top-level namespaces have a NULL parent.
CREATE TABLE namespaces (
    path text[] PRIMARY KEY CHECK (cardinality(path) >= 1),
    parent_path text[] GENERATED ALWAYS AS (
        CASE WHEN cardinality(path) > 1 THEN path[1:cardinality(path) - 1] END
    ) STORED REFERENCES namespaces (path) ON DELETE RESTRICT,
    properties jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX namespaces_parent_path_idx ON namespaces (parent_path);

-- Tables always live in a non-root namespace, so the foreign key also enforces that the
-- namespace exists at registration and blocks dropping a namespace that still has tables.
CREATE TABLE tables (
    namespace_path text[] NOT NULL REFERENCES namespaces (path) ON DELETE RESTRICT,
    name text NOT NULL,
    location text NOT NULL,
    properties jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (namespace_path, name)
);
