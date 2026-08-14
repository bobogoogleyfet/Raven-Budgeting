-- 003_utah_rates.sql — the published Utah rate table, refreshed each quarter.
--
-- SHIPPED MIGRATIONS ARE IMMUTABLE. See 001_init.sql.
--
-- # Why the rates become data
--
-- Until now the areas a user could pick from were a hand-written list in Rust,
-- forty-odd cities with rates typed in by hand. Utah publishes the real thing
-- — every taxing jurisdiction in the state — and revises it on the first day of
-- every quarter. A hand-written list is therefore wrong on a schedule, and a
-- release of this app is the only thing that could fix it.
--
-- Rates in a table can be replaced by a download instead of a release, which is
-- the only way an app can stay correct between one and the next.
--
-- # This does not rewrite history
--
-- `transaction.tax_rate_ppm` records the rate used when a receipt was entered
-- (001), and `location.tax_rate_ppm` records what a place charges (001, 002).
-- Neither is touched by a refresh. This table is the *prefill source*: it says
-- what Provo charges today, so picking Provo fills in today's figure. A
-- receipt from March keeps March's rate no matter how many quarters land after
-- it.

-- One row per taxing jurisdiction, straight from the state's workbook.
--
-- `code` is Utah's own "Cnty/City Code" (`25-029`), which is the stable
-- identity here: cities get renamed and re-cased in the source between
-- quarters, and matching on the name would lose a place's rate every time the
-- Tax Commission adjusts its spelling.
CREATE TABLE tax_area (
    id                INTEGER PRIMARY KEY,
    code              TEXT    NOT NULL UNIQUE,
    name              TEXT    NOT NULL,
    -- The published combined sales rate: state, county, city and every special
    -- district that applies there, already summed by the source.
    combined_rate_ppm INTEGER NOT NULL
        CHECK (combined_rate_ppm BETWEEN 0 AND 1000000),
    -- NULL where the state publishes no reduced rate for unprepared food. Utah
    -- publishes one flat 3% statewide, so in practice every row carries it —
    -- the column exists so a second state does not need a fourth migration.
    food_rate_ppm     INTEGER
        CHECK (food_rate_ppm IS NULL OR food_rate_ppm BETWEEN 0 AND 1000000),
    updated_at        TEXT    NOT NULL
        DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- The picker filters by what the user types, which is a name and never a code.
CREATE INDEX ix_tax_area_name ON tax_area (name);

-- Which jurisdiction a location's rates were taken from, when they were taken
-- from one at all.
--
-- This is what makes a quarterly refresh reach the user. Without it the table
-- could update forever and every receipt would still be taxed at whatever rate
-- was copied into `location` on the day it was created — the download would be
-- decoration.
--
-- NULL means the rate was typed by hand, and a refresh must not touch it: a
-- figure somebody read off a receipt outranks a published estimate, and
-- silently overwriting it each quarter would undo the same correction over and
-- over with nothing to show why.
--
-- Deliberately not a foreign key. `tax_area` is replaced wholesale every
-- quarter, and a reference would either block the delete or cascade it into the
-- user's own rows — a dissolved district must leave a location's last known
-- rate exactly where it is.
ALTER TABLE location ADD COLUMN area_code TEXT;

-- Which quarter's publication is loaded, and when we last managed to ask.
--
-- Single-row by construction: there is exactly one answer to "which rates are
-- these", and a table that could hold two would need a rule for which one wins.
CREATE TABLE tax_rate_release (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    -- `2026Q3`.
    quarter      TEXT NOT NULL,
    -- The day those rates take effect: the first of the quarter.
    effective_on TEXT NOT NULL,
    -- Where the file came from, so a stale table can be explained rather than
    -- just disbelieved.
    source_url   TEXT NOT NULL,
    installed_at TEXT NOT NULL,
    -- Last time a refresh was *attempted*, successful or not. Distinct from
    -- `installed_at`: a machine that has been offline for a month has checked
    -- recently and installed nothing, and telling those apart is what stops the
    -- app retrying a dead network on every keystroke.
    checked_at   TEXT
) STRICT;
