-- 006_budget_contribution.sql — money set aside is not money spent.
--
-- SHIPPED MIGRATIONS ARE IMMUTABLE. See 001_init.sql.
--
-- # Why this exists
--
-- A sinking fund only works if you actually put the money in. Until now the
-- app assumed you had: `funded_balance` computed contributions as
-- `monthly_contribution * months elapsed`, the *schedule*, so a yearly
-- insurance line reported itself funded on plan whether or not a cent had ever
-- been set aside. The one number the feature exists to answer — "will the money
-- be there when the bill lands" — was answered from the plan rather than from
-- reality.
--
-- The other half is what recording it must NOT do. Moving $100 into savings for
-- a yearly premium is not spending $100: filing it as a transaction would count
-- it against the category's budget, inflate every spending report, and then
-- count it a second time when the premium is actually paid. So a contribution
-- is its own kind of row and touches no transaction table. Nothing here is ever
-- read by a spending query.
--
-- # Why not budget_adjustment
--
-- 001 already has a table of per-category, per-month money. It means something
-- else: an adjustment is a *correction* — moving money between envelopes,
-- writing off an overspend so it stops propagating through every later rollover
-- balance. Those are edits to a plan that did not survive contact with the
-- month. A contribution is the plan working. Storing both in one table would
-- make "how much have I actually saved" unanswerable without a second column
-- saying which kind each row was, which is this table.

CREATE TABLE budget_contribution (
    id           INTEGER PRIMARY KEY,
    uuid         TEXT    NOT NULL UNIQUE,
    category_id  INTEGER NOT NULL REFERENCES category (id) ON DELETE RESTRICT,
    -- 'YYYY-MM'. The month the money was set aside in, not the month the bill
    -- is due: the whole point is to see the balance build up between them.
    year_month   TEXT    NOT NULL
                 CHECK (year_month GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]'),
    -- Signed, and deliberately not CHECKed positive. Taking money back out of
    -- a sinking fund is a thing that happens in a tight month, and recording it
    -- as a negative contribution keeps one running total instead of two.
    amount_cents INTEGER NOT NULL,
    notes        TEXT,
    archived_at  TEXT,
    created_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- One live contribution per category per month. Recording a second one for a
-- month you have already recorded is a correction to that figure, not a second
-- deposit — and two rows would silently double the funded balance, which is
-- exactly the number this table exists to make trustworthy.
CREATE UNIQUE INDEX ux_budget_contribution_month
    ON budget_contribution (category_id, year_month) WHERE archived_at IS NULL;

CREATE INDEX ix_budget_contribution_category
    ON budget_contribution (category_id, year_month);
