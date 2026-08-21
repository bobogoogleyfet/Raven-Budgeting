-- 008_sync.sql — what has changed here, and what has been seen from elsewhere.
--
-- SHIPPED MIGRATIONS ARE IMMUTABLE. See 001_init.sql.
--
-- # Why triggers rather than the write paths
--
-- The repositories are the only code that writes rows, and adding a change-log
-- line to each of them would be a handful of lines. It would also be a rule
-- that has to be remembered, once per write path, forever — and the failure
-- mode of forgetting is invisible: the row changes, nothing records it, and the
-- other device simply never learns. A trigger cannot be forgotten by a future
-- write path, because it is attached to the table rather than to the code.
--
-- # One row per entity, not one per edit
--
-- This is a *state-based* merge: a bundle carries the current value of whatever
-- changed, not the history of how it got there. So the log only needs to know
-- which entities are dirty and when they were last touched — editing a receipt
-- five times leaves one row, not five, and a device that has been offline for a
-- month sends one copy of each thing rather than a replay of the month.
--
-- # The aggregate is the unit
--
-- `txn_split` and `line_item` have no independent existence: the write path
-- deletes and re-inserts them wholesale inside one SQLite transaction. Logging
-- them separately would record dozens of changes for one edited receipt and
-- invite a merge that took splits from one device and the transaction from the
-- other — which is how you get a receipt whose splits do not sum to its total,
-- a state the schema's own CHECKs would refuse. Their triggers therefore mark
-- the parent `txn` dirty and say nothing about themselves.

-- ---------------------------------------------------------------------------
-- sync_clock — this device's identity and its hybrid logical clock.
-- ---------------------------------------------------------------------------
--
-- Single-row by construction, like `tax_rate_release`: there is exactly one
-- answer to "who am I and what time is it here", and a table that could hold
-- two would need a rule for which one wins.
--
-- `device` is empty until the app mints one. The triggers below still work: an
-- empty device makes the timestamp sort oddly against other devices, and a
-- database that has never been given an identity has also never synced, so
-- there is nothing to sort it against yet.
CREATE TABLE sync_clock (
    id      INTEGER PRIMARY KEY CHECK (id = 1),
    device  TEXT    NOT NULL DEFAULT '',
    -- Milliseconds since the epoch, never allowed to go backwards.
    millis  INTEGER NOT NULL DEFAULT 0,
    -- Disambiguates writes inside one millisecond. Saving a receipt writes the
    -- transaction and its splits well under a millisecond apart.
    counter INTEGER NOT NULL DEFAULT 0
) STRICT;

INSERT INTO sync_clock (id) VALUES (1);

-- ---------------------------------------------------------------------------
-- sync_change — which entities this device has touched, and when.
-- ---------------------------------------------------------------------------
CREATE TABLE sync_change (
    -- 'txn', 'category', 'location', 'merchant', 'budget_line', …
    entity      TEXT NOT NULL,
    -- The row's `uuid`: stable across devices, unlike its rowid (001).
    entity_uuid TEXT NOT NULL,
    -- The hybrid logical clock of the last local write, as
    -- '{millis:016x}-{counter:08x}-{device}'. Fixed-width hex so that TEXT
    -- order matches clock order — these are compared with `<` in SQL, where
    -- '9' > '10' would otherwise put a later edit before an earlier one.
    hlc         TEXT NOT NULL,
    PRIMARY KEY (entity, entity_uuid)
) STRICT;

-- Bundling asks "everything since the last time I sent to this peer", which is
-- a range scan over the clock and nothing else.
CREATE INDEX ix_sync_change_hlc ON sync_change (hlc);

-- ---------------------------------------------------------------------------
-- sync_peer — how far along we are with each other device.
-- ---------------------------------------------------------------------------
CREATE TABLE sync_peer (
    device        TEXT PRIMARY KEY,
    -- The high-water mark of what we have sent them, so the next bundle
    -- carries only what has changed since.
    sent_through  TEXT,
    -- The high-water mark of what we have applied from them, so a bundle
    -- re-imported (the same file dropped in twice) is cheap rather than wrong.
    seen_through  TEXT,
    -- Names the last exchange, for a screen that has to explain why two
    -- devices disagree.
    synced_at     TEXT
) STRICT;

-- ---------------------------------------------------------------------------
-- sync_conflict — the version that lost, kept rather than discarded.
-- ---------------------------------------------------------------------------
--
-- Last-writer-wins is a rule for picking a value, not a claim that the other
-- value was wrong. Two people entering receipts will occasionally edit the same
-- thing, and silently dropping one person's figure — with the app showing no
-- sign it ever existed — is the failure that makes a shared ledger untrusted.
-- So the loser is kept, verbatim, and the row it belongs to is flagged for
-- review through the inbox that already exists for exactly this purpose.
CREATE TABLE sync_conflict (
    id          INTEGER PRIMARY KEY,
    entity      TEXT    NOT NULL,
    entity_uuid TEXT    NOT NULL,
    -- The losing side's whole aggregate, as it arrived. Opaque here on
    -- purpose: this table is a receipt for a decision, not a second schema,
    -- and it must survive the winning side's shape changing under it.
    payload     TEXT    NOT NULL,
    -- Whose it was, and when they wrote it.
    hlc         TEXT    NOT NULL,
    noticed_at  TEXT    NOT NULL
                DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- Set once somebody has looked. Not deleted: the record that the two
    -- devices once disagreed is the useful part.
    resolved_at TEXT
) STRICT;

CREATE INDEX ix_sync_conflict_open
    ON sync_conflict (entity, entity_uuid) WHERE resolved_at IS NULL;

-- ---------------------------------------------------------------------------
-- The triggers.
-- ---------------------------------------------------------------------------
--
-- Each one does the same two things: advance this device's clock by one tick,
-- then stamp the touched entity with the result.
--
-- The clock advance is the hybrid-logical-clock rule written in SQL. `millis`
-- takes the larger of the wall clock and what it already held, so a clock
-- correction — or a laptop waking up — cannot make an edit sort before one
-- already recorded. The counter resets when the millisecond genuinely advances
-- and otherwise increments, which is what keeps two writes inside the same
-- millisecond in the order they happened.
--
-- SQLite evaluates every expression in an UPDATE's SET against the row's
-- *old* values, so `millis` and `counter` below both read the pre-update
-- state. Writing this as two statements would be a race with itself.
--
-- The upsert keeps one row per entity: an entity edited five times is still
-- one thing to send.


CREATE TRIGGER trg_sync_category_group_insert
AFTER INSERT ON category_group
FOR EACH ROW
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'category_group', NEW.uuid,
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_category_group_update
AFTER UPDATE ON category_group
FOR EACH ROW
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'category_group', NEW.uuid,
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_category_insert
AFTER INSERT ON category
FOR EACH ROW
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'category', NEW.uuid,
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_category_update
AFTER UPDATE ON category
FOR EACH ROW
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'category', NEW.uuid,
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_location_insert
AFTER INSERT ON location
FOR EACH ROW
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'location', NEW.uuid,
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_location_update
AFTER UPDATE ON location
FOR EACH ROW
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'location', NEW.uuid,
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_merchant_insert
AFTER INSERT ON merchant
FOR EACH ROW
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'merchant', NEW.uuid,
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_merchant_update
AFTER UPDATE ON merchant
FOR EACH ROW
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'merchant', NEW.uuid,
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_txn_insert
AFTER INSERT ON txn
FOR EACH ROW
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'txn', NEW.uuid,
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_txn_update
AFTER UPDATE ON txn
FOR EACH ROW
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'txn', NEW.uuid,
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_budget_line_insert
AFTER INSERT ON budget_line
FOR EACH ROW
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'budget_line', NEW.uuid,
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_budget_line_update
AFTER UPDATE ON budget_line
FOR EACH ROW
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'budget_line', NEW.uuid,
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_budget_adjustment_insert
AFTER INSERT ON budget_adjustment
FOR EACH ROW
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'budget_adjustment', NEW.uuid,
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_budget_adjustment_update
AFTER UPDATE ON budget_adjustment
FOR EACH ROW
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'budget_adjustment', NEW.uuid,
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_budget_contribution_insert
AFTER INSERT ON budget_contribution
FOR EACH ROW
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'budget_contribution', NEW.uuid,
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_budget_contribution_update
AFTER UPDATE ON budget_contribution
FOR EACH ROW
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'budget_contribution', NEW.uuid,
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

-- `txn_split` and `line_item` mark the receipt they belong to. Logging them
-- as entities of their own would record dozens of changes for one edited
-- receipt, and would invite a merge that took the splits from one device and
-- the transaction from the other.
--
-- DELETE is included where the others are not, because these two rows *are*
-- hard-deleted: the write path replaces a whole aggregate by deleting the
-- children and re-inserting them. Removing the last split from a receipt is a
-- real change to it, and without this it would be the one edit that never
-- travelled.

CREATE TRIGGER trg_sync_txn_split_insert
AFTER INSERT ON txn_split
FOR EACH ROW
-- Silent when the parent is going too: deleting a receipt cascades to its
-- children, and stamping a transaction that no longer exists would put a
-- phantom in the bundle.
WHEN (SELECT uuid FROM txn WHERE id = NEW.txn_id) IS NOT NULL
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'txn', (SELECT uuid FROM txn WHERE id = NEW.txn_id),
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_txn_split_update
AFTER UPDATE ON txn_split
FOR EACH ROW
-- Silent when the parent is going too: deleting a receipt cascades to its
-- children, and stamping a transaction that no longer exists would put a
-- phantom in the bundle.
WHEN (SELECT uuid FROM txn WHERE id = NEW.txn_id) IS NOT NULL
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'txn', (SELECT uuid FROM txn WHERE id = NEW.txn_id),
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_txn_split_delete
AFTER DELETE ON txn_split
FOR EACH ROW
-- Silent when the parent is going too: deleting a receipt cascades to its
-- children, and stamping a transaction that no longer exists would put a
-- phantom in the bundle.
WHEN (SELECT uuid FROM txn WHERE id = OLD.txn_id) IS NOT NULL
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'txn', (SELECT uuid FROM txn WHERE id = OLD.txn_id),
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_line_item_insert
AFTER INSERT ON line_item
FOR EACH ROW
-- Silent when the parent is going too: deleting a receipt cascades to its
-- children, and stamping a transaction that no longer exists would put a
-- phantom in the bundle.
WHEN (SELECT t.uuid FROM txn t JOIN txn_split s ON s.txn_id = t.id WHERE s.id = NEW.split_id) IS NOT NULL
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'txn', (SELECT t.uuid FROM txn t JOIN txn_split s ON s.txn_id = t.id WHERE s.id = NEW.split_id),
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_line_item_update
AFTER UPDATE ON line_item
FOR EACH ROW
-- Silent when the parent is going too: deleting a receipt cascades to its
-- children, and stamping a transaction that no longer exists would put a
-- phantom in the bundle.
WHEN (SELECT t.uuid FROM txn t JOIN txn_split s ON s.txn_id = t.id WHERE s.id = NEW.split_id) IS NOT NULL
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'txn', (SELECT t.uuid FROM txn t JOIN txn_split s ON s.txn_id = t.id WHERE s.id = NEW.split_id),
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;

CREATE TRIGGER trg_sync_line_item_delete
AFTER DELETE ON line_item
FOR EACH ROW
-- Silent when the parent is going too: deleting a receipt cascades to its
-- children, and stamping a transaction that no longer exists would put a
-- phantom in the bundle.
WHEN (SELECT t.uuid FROM txn t JOIN txn_split s ON s.txn_id = t.id WHERE s.id = OLD.split_id) IS NOT NULL
BEGIN
    UPDATE sync_clock
       SET millis  = MAX(millis, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)),
           counter = CASE
                         WHEN CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) > millis
                         THEN 0 ELSE counter + 1
                     END
     WHERE id = 1;
    INSERT INTO sync_change (entity, entity_uuid, hlc)
    SELECT 'txn', (SELECT t.uuid FROM txn t JOIN txn_split s ON s.txn_id = t.id WHERE s.id = OLD.split_id),
           printf('%016x-%08x-%s', millis, counter, device)
      FROM sync_clock WHERE id = 1
        ON CONFLICT (entity, entity_uuid)
        DO UPDATE SET hlc = excluded.hlc;
END;
