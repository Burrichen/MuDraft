-- v5: tag colours and protected built-in tags.

-- Lowercase #rrggbb. Chip text colour is chosen for contrast by the UI.
ALTER TABLE tag ADD COLUMN color TEXT NOT NULL DEFAULT '#64748b'
    CHECK (color GLOB '#[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]');

-- The built-in Listen ASAP tag keeps a stable identity (seeded once in v2 with a fixed ID).
UPDATE tag SET color = '#f59e0b' WHERE builtin_key = 'listen_asap';

-- Built-in tags can change colour and assignments, but never name, key, or existence.
CREATE TRIGGER tag_builtin_no_delete BEFORE DELETE ON tag
WHEN OLD.builtin_key IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'built-in tags cannot be deleted');
END;

CREATE TRIGGER tag_builtin_no_rename BEFORE UPDATE OF name, builtin_key, id ON tag
WHEN OLD.builtin_key IS NOT NULL
    AND (NEW.name IS NOT OLD.name OR NEW.builtin_key IS NOT OLD.builtin_key OR NEW.id IS NOT OLD.id)
BEGIN
    SELECT RAISE(ABORT, 'built-in tags cannot be renamed');
END;
