-- v15: the built-in Listen ASAP tag may be deleted. New libraries still start with it
-- (seeded once in v2); once deleted it stays deleted. Its name and key stay fixed while it
-- exists (tag_builtin_no_rename), so Weighted Random can always find it by key.

DROP TRIGGER tag_builtin_no_delete;
