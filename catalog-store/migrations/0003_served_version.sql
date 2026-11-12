-- The version unversioned reads answer for, and its manifest e-tag. NULL until registration,
-- a refresh, or the first read records the newest version.
ALTER TABLE tables
    ADD COLUMN served_version bigint,
    ADD COLUMN served_e_tag text;
