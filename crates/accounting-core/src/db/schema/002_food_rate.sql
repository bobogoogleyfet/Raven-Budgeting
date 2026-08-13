-- 002_food_rate.sql — a second tax rate, for categories taxed at a reduced one.
--
-- SHIPPED MIGRATIONS ARE IMMUTABLE. See 001_init.sql.
--
-- # Why this exists
--
-- A single rate per location is wrong in most US states. Utah taxes unprepared
-- food at a combined 3% while everything else is around 7.25%, so a $7.28
-- grocery subtotal owes $0.22, not the $0.53 one rate would predict — a 140%
-- overestimate on an ordinary receipt.
--
-- `location_category_rate` (in 001) can already express a per-category
-- override, and it stays the right tool for a genuinely local exception. It is
-- the wrong tool for this: the reduced food rate applies to a *class* of
-- category at *every* location in the state, so expressing it there would mean
-- writing one row per (location × food category) and re-writing them all every
-- time a location is added. The rule is "this category is food" and "food is
-- taxed at this rate here", which is what these two columns say.
--
-- Both are nullable/defaulted so 001-era databases keep working unchanged: a
-- NULL food rate means "no reduced rate here", which is the correct reading for
-- every state that does not have one.

-- NULL means this location has no reduced rate; food is taxed like everything
-- else. Not defaulted to 0, which would mean "food is tax-free here" — a very
-- different and much more wrong claim.
ALTER TABLE location ADD COLUMN food_rate_ppm INTEGER
    CHECK (food_rate_ppm IS NULL OR food_rate_ppm BETWEEN 0 AND 1000000);

-- Whether this category is the kind of thing a reduced food rate applies to.
-- Groceries yes; Restaurants no, because prepared food is taxed in full.
ALTER TABLE category ADD COLUMN uses_food_rate INTEGER NOT NULL DEFAULT 0
    CHECK (uses_food_rate IN (0, 1));
