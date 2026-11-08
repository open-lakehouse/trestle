-- Schema version 2: a revision's commit message (none for revisions committed before).

ALTER TABLE revision ADD COLUMN message TEXT;
