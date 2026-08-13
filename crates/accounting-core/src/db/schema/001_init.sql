-- 001_init.sql — the initial schema.
--
-- SHIPPED MIGRATIONS ARE IMMUTABLE. Once this file has run against a real
-- database, editing it silently diverges the schema on new installs from the
-- schema on existing ones. Add 002_*.sql instead. There is an integration test
-- that hashes this file for exactly that reason.
--
-- Conventions used throughout:
--
--   * Every table is STRICT. This is the whole point: in a non-STRICT table a
--     stray '12.34' lands in an INTEGER cents column as TEXT and every later
--     SUM() is quietly wrong. STRICT makes it an error at write time.
--   * Money is INTEGER cents. Never REAL, never TEXT.
--   * Tax rates are INTEGER parts-per-million: 9.4375% is exactly 94375.
--     Percent-with-2-decimals rounds that away; a float is differently wrong.
--   * Booleans are INTEGER 0/1 with a CHECK, because STRICT has no BOOLEAN.
--   * Dates are TEXT: 'YYYY-MM-DD' for days, 'YYYY-MM' for budget periods,
--     and ISO-8601 UTC for timestamps. Validated with GLOB rather than
--     date(), which SQLite rejects in a CHECK as non-deterministic.
--   * created_at / updated_at default to UTC now. That is SQLite reading the
--     clock, not core Rust: it keeps the write paths short and it avoids
--     chrono's local-timezone lookup, which needs JNI on Android.
--   * archived_at marks a row dead. Nothing user-facing is ever hard-deleted;
--     historical transactions must keep resolving their category names.
--   * sort_order values are SPARSE (100, 200, 300 ...) so a drag-reorder
--     rewrites one row instead of every row.
--   * uuid is the stable cross-device identity; the INTEGER PRIMARY KEY is the
--     rowid and is local to one file.

-- ---------------------------------------------------------------------------
-- app_setting — key/value scalars (seed version, report basis, ...).
-- ---------------------------------------------------------------------------
CREATE TABLE app_setting (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- ---------------------------------------------------------------------------
-- category_group / category — ONE shared taxonomy, exactly one level of
-- nesting. Budget lines and transaction splits both point at `category`, so
-- actuals align with budgets for free: SUM(txn_split.amount_cents) GROUP BY
-- category_id. Two parallel taxonomies would make that a reconciliation
-- problem instead of a GROUP BY.
-- ---------------------------------------------------------------------------
CREATE TABLE category_group (
    id          INTEGER PRIMARY KEY,
    uuid        TEXT    NOT NULL UNIQUE,
    name        TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    -- Hidden groups exist but are not offered in the category picker; the
    -- System group is hidden so Uncategorized/Transfer/Reimbursable do not
    -- clutter the user's own structure.
    is_hidden   INTEGER NOT NULL DEFAULT 0 CHECK (is_hidden IN (0, 1)),
    sort_order  INTEGER NOT NULL DEFAULT 1000,
    archived_at TEXT,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- Partial: an archived group's name is released for reuse.
CREATE UNIQUE INDEX ux_category_group_name
    ON category_group (name) WHERE archived_at IS NULL;

CREATE TABLE category (
    id          INTEGER PRIMARY KEY,
    uuid        TEXT    NOT NULL UNIQUE,
    group_id    INTEGER NOT NULL REFERENCES category_group (id) ON DELETE RESTRICT,
    name        TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    -- NULL for user categories. Non-NULL marks a category the application
    -- itself depends on (see the triggers below): 'uncategorized' is the
    -- fallback that makes quick-add possible, 'transfer' keeps account moves
    -- out of spending reports, 'reimbursable' parks expenses awaiting payback.
    system_key  TEXT UNIQUE CHECK (system_key IS NULL OR length(system_key) > 0),
    -- Excluded from budget screens (system categories, tracking-only ones).
    is_budgeted INTEGER NOT NULL DEFAULT 1 CHECK (is_budgeted IN (0, 1)),
    -- Seeds budget_line.rollover for new lines. Non-monthly categories need
    -- it: without rollover a sinking fund is impossible.
    default_rollover INTEGER NOT NULL DEFAULT 0 CHECK (default_rollover IN (0, 1)),
    sort_order  INTEGER NOT NULL DEFAULT 1000,
    archived_at TEXT,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- Names are unique within a group, not globally: "Insurance" under Bills and
-- under Non-Monthly are different envelopes and both are reasonable.
CREATE UNIQUE INDEX ux_category_name
    ON category (group_id, name) WHERE archived_at IS NULL;
CREATE INDEX ix_category_group ON category (group_id, sort_order);

-- System categories are load-bearing. Deleting Uncategorized would strand
-- every quick-add row; archiving it would hide the needs-categorizing inbox.
CREATE TRIGGER trg_category_system_no_delete
BEFORE DELETE ON category
FOR EACH ROW WHEN OLD.system_key IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'system categories cannot be deleted');
END;

CREATE TRIGGER trg_category_system_no_archive
BEFORE UPDATE OF archived_at ON category
FOR EACH ROW WHEN OLD.system_key IS NOT NULL AND NEW.archived_at IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'system categories cannot be archived');
END;

-- Otherwise the previous two triggers are trivially defeated by first
-- clearing system_key.
CREATE TRIGGER trg_category_system_key_immutable
BEFORE UPDATE OF system_key ON category
FOR EACH ROW WHEN OLD.system_key IS NOT NULL
     AND (NEW.system_key IS NULL OR NEW.system_key <> OLD.system_key)
BEGIN
    SELECT RAISE(ABORT, 'system_key is immutable');
END;

-- A system category may be renamed and re-parented, but not moved out from
-- under the application's feet by archiving its group.
CREATE TRIGGER trg_category_group_no_archive_system
BEFORE UPDATE OF archived_at ON category_group
FOR EACH ROW WHEN NEW.archived_at IS NOT NULL AND OLD.archived_at IS NULL
     AND EXISTS (SELECT 1 FROM category
                 WHERE group_id = OLD.id AND system_key IS NOT NULL)
BEGIN
    SELECT RAISE(ABORT, 'a group holding system categories cannot be archived');
END;

-- ---------------------------------------------------------------------------
-- location — where the STORE is, which is what determines sales tax. Not where
-- the user is, and emphatically not derived from a ZIP code: ZIPs are USPS
-- carrier routes and do not align with the ~13k US taxing jurisdictions.
--
-- The rate here only PREFILLS an editable estimate. The receipt prints the
-- real tax and the real tax always wins.
-- ---------------------------------------------------------------------------
CREATE TABLE location (
    id                 INTEGER PRIMARY KEY,
    uuid               TEXT    NOT NULL UNIQUE,
    name               TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    -- Parts per million. 8.375% = 83750. 0 .. 1000000 (= 0% .. 100%).
    tax_rate_ppm       INTEGER NOT NULL DEFAULT 0
                       CHECK (tax_rate_ppm BETWEEN 0 AND 1000000),
    -- Shelf prices already include tax (rare in the US, normal under VAT).
    -- Present from day one so VAT is a data change, not a rewrite.
    prices_include_tax INTEGER NOT NULL DEFAULT 0
                       CHECK (prices_include_tax IN (0, 1)),
    is_home            INTEGER NOT NULL DEFAULT 0 CHECK (is_home IN (0, 1)),
    notes              TEXT,
    archived_at        TEXT,
    created_at         TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at         TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE UNIQUE INDEX ux_location_name
    ON location (name) WHERE archived_at IS NULL;
-- At most one home location. A second one is an entry bug, not a preference.
CREATE UNIQUE INDEX ux_location_home
    ON location (is_home) WHERE is_home = 1 AND archived_at IS NULL;

-- Per-category rate overrides: groceries are exempt in ~32 states, and a
-- single per-location rate cannot express that.
CREATE TABLE location_category_rate (
    location_id  INTEGER NOT NULL REFERENCES location (id) ON DELETE CASCADE,
    category_id  INTEGER NOT NULL REFERENCES category (id) ON DELETE CASCADE,
    tax_rate_ppm INTEGER NOT NULL CHECK (tax_rate_ppm BETWEEN 0 AND 1000000),
    updated_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (location_id, category_id)
) STRICT;

-- ---------------------------------------------------------------------------
-- merchant — the store-memory object. Remembering category, location and
-- detail level per store is what removes three of the six decisions the naive
-- entry flow demanded, and six-decisions-before-a-dollar is the documented
-- reason manual trackers get abandoned in 3-4 weeks.
--
-- Deliberately NO last_txn_id column. SQLite cannot ALTER TABLE ADD
-- CONSTRAINT, so a merchant -> txn -> merchant FK cycle becomes a table
-- creation-order problem forever. "Repeat last receipt" is a query against
-- ix_txn_merchant_date, which is O(log n) and can never go stale.
-- ---------------------------------------------------------------------------
CREATE TABLE merchant (
    id                   INTEGER PRIMARY KEY,
    uuid                 TEXT    NOT NULL UNIQUE,
    name                 TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    default_category_id  INTEGER REFERENCES category (id) ON DELETE SET NULL,
    default_location_id  INTEGER REFERENCES location (id) ON DELETE SET NULL,
    -- Per-store, not a global first-run mode: you itemize the grocery run and
    -- not the gas station, and that preference is stable per store.
    default_detail_level TEXT    NOT NULL DEFAULT 'grouped'
                         CHECK (default_detail_level IN ('grouped', 'itemized')),
    -- Drives recent-store chips. Written by the app, never read as "today".
    last_used_at         TEXT,
    notes                TEXT,
    archived_at          TEXT,
    created_at           TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at           TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE UNIQUE INDEX ux_merchant_name
    ON merchant (name) WHERE archived_at IS NULL;
CREATE INDEX ix_merchant_recent ON merchant (last_used_at DESC);

-- ---------------------------------------------------------------------------
-- txn -> txn_split -> line_item. ONE model at two detail levels, not two
-- modes: "grouped" is a split with no line items, "itemized" is a split
-- expanded into them. Every report reads txn_split and is indifferent.
-- ---------------------------------------------------------------------------
CREATE TABLE txn (
    id               INTEGER PRIMARY KEY,
    uuid             TEXT    NOT NULL UNIQUE,
    -- 'YYYY-MM-DD'. Actuals are always cash-flow on this date, in every
    -- reporting basis, so the numbers can be reconciled against a bank
    -- statement.
    txn_date         TEXT    NOT NULL
                     CHECK (txn_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    merchant_id      INTEGER REFERENCES merchant (id) ON DELETE RESTRICT,
    location_id      INTEGER REFERENCES location (id) ON DELETE RESTRICT,
    subtotal_cents   INTEGER NOT NULL,
    tax_cents        INTEGER NOT NULL,
    total_cents      INTEGER NOT NULL,
    -- True only when the tax was computed rather than read off the receipt.
    -- Surfaced in the UI and exported, because an estimate that silently
    -- looks like a fact is worse than no estimate.
    tax_is_estimated INTEGER NOT NULL DEFAULT 0
                     CHECK (tax_is_estimated IN (0, 1)),
    -- The rate actually used at entry time. Kept per-row: the location's rate
    -- changes, and last year's receipt must not change with it.
    tax_rate_ppm     INTEGER CHECK (tax_rate_ppm IS NULL
                                    OR tax_rate_ppm BETWEEN 0 AND 1000000),
    detail_level     TEXT    NOT NULL DEFAULT 'grouped'
                     CHECK (detail_level IN ('grouped', 'itemized')),
    -- Set by quick-add; drives the needs-categorizing inbox.
    needs_review     INTEGER NOT NULL DEFAULT 0 CHECK (needs_review IN (0, 1)),
    notes            TEXT,
    archived_at      TEXT,
    created_at       TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at       TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- Row-local, so it is enforceable as a CHECK. The two AGGREGATE
    -- invariants (splits sum to total, items sum to split) cannot be, because
    -- CHECK cannot aggregate -- see db::integrity.
    --
    -- SQLite requires table constraints to follow every column definition;
    -- one written between two columns is a syntax error, which is why these
    -- sit at the bottom rather than next to the columns they constrain.
    CHECK (subtotal_cents + tax_cents = total_cents)
) STRICT;

CREATE INDEX ix_txn_date ON txn (txn_date DESC);
-- The "repeat last receipt" index, and the reason merchant.last_txn_id does
-- not exist.
CREATE INDEX ix_txn_merchant_date ON txn (merchant_id, txn_date DESC);
CREATE INDEX ix_txn_needs_review ON txn (needs_review) WHERE needs_review = 1;

CREATE TABLE txn_split (
    id             INTEGER PRIMARY KEY,
    uuid           TEXT    NOT NULL UNIQUE,
    -- CASCADE, not RESTRICT: splits have no independent existence. The write
    -- path replaces the whole aggregate (delete children, re-insert) inside
    -- one SQLite transaction, which is what keeps the aggregate invariants
    -- true without mid-edit triggers.
    txn_id         INTEGER NOT NULL REFERENCES txn (id) ON DELETE CASCADE,
    category_id    INTEGER NOT NULL REFERENCES category (id) ON DELETE RESTRICT,
    subtotal_cents INTEGER NOT NULL,
    -- Tax is allocated across splits proportionally by Money::allocate, never
    -- computed per split and summed -- that drifts a penny per split.
    tax_cents      INTEGER NOT NULL,
    amount_cents   INTEGER NOT NULL,
    sort_order     INTEGER NOT NULL DEFAULT 100,
    notes          TEXT,
    CHECK (subtotal_cents + tax_cents = amount_cents)
) STRICT;

CREATE INDEX ix_txn_split_txn ON txn_split (txn_id, sort_order);
CREATE INDEX ix_txn_split_category ON txn_split (category_id);

CREATE TABLE line_item (
    id               INTEGER PRIMARY KEY,
    uuid             TEXT    NOT NULL UNIQUE,
    split_id         INTEGER NOT NULL REFERENCES txn_split (id) ON DELETE CASCADE,
    description      TEXT    NOT NULL CHECK (length(trim(description)) > 0),
    -- Thousandths of a unit, so 1.5 lb of apples is 1500. Integer, because a
    -- REAL quantity multiplied by a price is how you get $3.9299999.
    quantity_milli   INTEGER NOT NULL DEFAULT 1000 CHECK (quantity_milli > 0),
    unit_price_cents INTEGER,
    subtotal_cents   INTEGER NOT NULL,
    tax_cents        INTEGER NOT NULL,
    amount_cents     INTEGER NOT NULL,
    -- Optional per-item category override, for the one non-food item in a
    -- grocery run. NULL means "same as the split".
    category_id      INTEGER REFERENCES category (id) ON DELETE RESTRICT,
    is_taxable       INTEGER NOT NULL DEFAULT 1 CHECK (is_taxable IN (0, 1)),
    sort_order       INTEGER NOT NULL DEFAULT 100,
    notes            TEXT,
    CHECK (subtotal_cents + tax_cents = amount_cents)
) STRICT;

CREATE INDEX ix_line_item_split ON line_item (split_id, sort_order);
CREATE INDEX ix_line_item_category ON line_item (category_id);

-- ---------------------------------------------------------------------------
-- budget_line — a budget for one category over one closed or open period.
--
-- period_start/period_end exist so that changing next year's budget does not
-- rewrite last year's report. A budget is a historical record, not a setting.
-- ---------------------------------------------------------------------------
CREATE TABLE budget_line (
    id              INTEGER PRIMARY KEY,
    uuid            TEXT    NOT NULL UNIQUE,
    category_id     INTEGER NOT NULL REFERENCES category (id) ON DELETE RESTRICT,
    period_type     TEXT    NOT NULL
                    CHECK (period_type IN ('monthly', 'yearly',
                                           'every_n_months', 'by_date')),
    -- Only meaningful for 'every_n_months'.
    period_interval INTEGER,
    -- 'YYYY-MM'. The first month this line is in force.
    period_start    TEXT    NOT NULL
                    CHECK (period_start GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]'),
    -- 'YYYY-MM' inclusive, or NULL for "still in force".
    period_end      TEXT
                    CHECK (period_end IS NULL
                           OR (period_end GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]'
                               AND period_end >= period_start)),
    amount_cents    INTEGER NOT NULL,
    -- Required for non-monthly categories: without carry-over, a sinking fund
    -- is impossible and the whole yearly-expense feature collapses.
    rollover        INTEGER NOT NULL DEFAULT 0 CHECK (rollover IN (0, 1)),
    -- 'YYYY-MM-DD', when the bill is actually due. Required for 'by_date'.
    due_date        TEXT
                    CHECK (due_date IS NULL
                           OR due_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    notes           TEXT,
    archived_at     TEXT,
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- Table constraints, at the bottom because SQLite rejects one written
    -- between two column definitions.
    CHECK (period_type <> 'every_n_months'
           OR (period_interval IS NOT NULL AND period_interval BETWEEN 2 AND 120)),
    CHECK (period_type <> 'by_date' OR due_date IS NOT NULL)
) STRICT;

-- One live line per category per starting month. Fully general non-overlap
-- cannot be expressed as an index; the repository layer enforces the rest.
CREATE UNIQUE INDEX ux_budget_line_period
    ON budget_line (category_id, period_start) WHERE archived_at IS NULL;
CREATE INDEX ix_budget_line_category ON budget_line (category_id, period_start);

-- Manual corrections to a category's envelope for one month: moving money
-- between envelopes, or writing off an overspend so it stops propagating
-- through every subsequent rollover balance.
CREATE TABLE budget_adjustment (
    id           INTEGER PRIMARY KEY,
    uuid         TEXT    NOT NULL UNIQUE,
    category_id  INTEGER NOT NULL REFERENCES category (id) ON DELETE RESTRICT,
    year_month   TEXT    NOT NULL
                 CHECK (year_month GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]'),
    amount_cents INTEGER NOT NULL,
    reason       TEXT,
    archived_at  TEXT,
    created_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX ix_budget_adjustment_category
    ON budget_adjustment (category_id, year_month);
