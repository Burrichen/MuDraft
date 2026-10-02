-- v11: the built-in Listen ASAP tag's default colour becomes mustard yellow.
-- Only the untouched old default changes; a colour the user picked is kept.
UPDATE tag SET color = '#d4a017' WHERE builtin_key = 'listen_asap' AND color = '#f59e0b';
