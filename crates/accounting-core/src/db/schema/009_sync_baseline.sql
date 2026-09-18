-- 009_sync_baseline.sql — log every row that existed before the change log did.
--
-- SHIPPED MIGRATIONS ARE IMMUTABLE. See 001_init.sql.
--
-- # The hole this fills
--
-- 008 added triggers that record every write in `sync_change`, and a bundle is
-- built from that log and nothing else. The triggers only see writes made
-- after they existed. Every row a database already held when 008 ran — on a
-- desktop that had been in use for months, most of the ledger — was never
-- logged, so no bundle could ever carry it. Ordinary sync between two copies
-- of the same file hid this, because both copies already had those rows. A
-- device *joining* did not: it threw its own seed away and received only the
-- handful of rows edited since 008, which is a ledger missing most of its
-- receipts, most of its categories, and its home location.
--
-- # Why the stamp is a fixed, device-less "before sync began"
--
-- Each backfilled entry gets the same timestamp on every device:
-- `0000000000000000-00000000-(baseline)`. Every part of that is deliberate.
--
-- * Time zero sorts before every real edit anywhere. A backfilled row must
--   never outrank somebody's actual change to it, and it must never look like
--   something newer than a watermark two paired devices have already agreed
--   on — so an exchange between devices that were already syncing sends none
--   of these rows, and changes nothing. Only a device that has never been sent
--   anything (a watermark of nothing) receives them, which is exactly the
--   device that needs them.
--
-- * No device. This migration can run before the database has a name — an
--   old file upgraded straight past 008 names itself only after migrating —
--   and an entry stamped with an empty name is one the bundle reader rejects,
--   silently. More to the point, nobody *wrote* these rows at a moment sync
--   can see. Naming this device would make its merge treat each one as its
--   own edit, so a correction made on a device that joined from here would be
--   recorded here as a disagreement.
--
-- * The same text everywhere. Two copies of one database — a backup restored
--   onto a phone, which is how the first real pair was made — each backfill
--   the same rows with the same stamp. If those ever do meet, the stamps are
--   equal, so neither side replaces the other's identical row and neither
--   records a conflict.
--
-- All rows, archived ones included. A delete is a soft `archived_at`, and an
-- old receipt still points at the archived category it was filed under; a
-- joiner that received the receipt without the category could not apply it.
--
-- `txn_split` and `line_item` travel inside their receipt (008), so logging
-- the receipt is logging them.
--
-- `OR IGNORE` keeps every entry the triggers already wrote: those are real
-- edits with real times, and replacing one with time zero would stop it from
-- ever being sent again.

INSERT OR IGNORE INTO sync_change (entity, entity_uuid, hlc)
SELECT 'category_group', uuid, '0000000000000000-00000000-(baseline)' FROM category_group;

INSERT OR IGNORE INTO sync_change (entity, entity_uuid, hlc)
SELECT 'category', uuid, '0000000000000000-00000000-(baseline)' FROM category;

INSERT OR IGNORE INTO sync_change (entity, entity_uuid, hlc)
SELECT 'location', uuid, '0000000000000000-00000000-(baseline)' FROM location;

INSERT OR IGNORE INTO sync_change (entity, entity_uuid, hlc)
SELECT 'merchant', uuid, '0000000000000000-00000000-(baseline)' FROM merchant;

INSERT OR IGNORE INTO sync_change (entity, entity_uuid, hlc)
SELECT 'budget_line', uuid, '0000000000000000-00000000-(baseline)' FROM budget_line;

INSERT OR IGNORE INTO sync_change (entity, entity_uuid, hlc)
SELECT 'budget_adjustment', uuid, '0000000000000000-00000000-(baseline)' FROM budget_adjustment;

INSERT OR IGNORE INTO sync_change (entity, entity_uuid, hlc)
SELECT 'budget_contribution', uuid, '0000000000000000-00000000-(baseline)' FROM budget_contribution;

INSERT OR IGNORE INTO sync_change (entity, entity_uuid, hlc)
SELECT 'txn', uuid, '0000000000000000-00000000-(baseline)' FROM txn;
