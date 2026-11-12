-- Renamed to "current version" to match the terminology used elsewhere in the catalog.
ALTER TABLE tables
    RENAME COLUMN served_version TO current_version;
ALTER TABLE tables
    RENAME COLUMN served_e_tag TO current_e_tag;
