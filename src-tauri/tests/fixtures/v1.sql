-- A database as written by schema v1 (the foundation build), with user data in it.
CREATE TABLE app_meta (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;
INSERT INTO app_meta (key, value) VALUES ('installed_by', 'mudraft 0.1.0');
PRAGMA user_version = 1;
