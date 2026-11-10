-- Catalog-recorded table facts get dedicated columns. Owner and description move out of the
-- properties JSON; describe folds them back in, so the wire contract is unchanged.
ALTER TABLE tables RENAME COLUMN created_at TO registered_at;

ALTER TABLE tables
    ADD COLUMN updated_at timestamptz,
    ADD COLUMN owner text,
    ADD COLUMN description text;

-- Existing tables were never re-registered as far as the catalog knows.
UPDATE tables SET
    updated_at = registered_at,
    owner = properties ->> 'owner',
    description = properties ->> 'description',
    properties = properties - 'owner' - 'description';

ALTER TABLE tables
    ALTER COLUMN updated_at SET NOT NULL,
    ALTER COLUMN updated_at SET DEFAULT now();
