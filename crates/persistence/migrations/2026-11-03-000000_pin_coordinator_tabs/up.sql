-- Coordinators were always restored as pinned, without saving it. Pins now come from this column,
-- so a coordinator the user unpins stays unpinned.
UPDATE tabs SET pinned = TRUE WHERE coordinator = TRUE;
