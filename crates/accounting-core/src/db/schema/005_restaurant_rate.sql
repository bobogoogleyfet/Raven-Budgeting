-- 005_restaurant_rate.sql — prepared food is taxed ABOVE the general rate.
--
-- SHIPPED MIGRATIONS ARE IMMUTABLE. See 001_init.sql.
--
-- # Why this exists
--
-- 002 taught the app that some categories are taxed *below* the general rate:
-- Utah charges a combined 3% on unprepared food against roughly 7.45% on
-- everything else. It had no way to say the opposite, and the opposite is also
-- true. Utah counties may levy a Restaurant Tax of up to 1% on prepared food
-- (Utah Code 59-12-603), on top of the combined rate.
--
-- The symptom was a receipt that did not reconcile. A $13.59 restaurant
-- subtotal in Highland owes $1.15, not the $1.01 that 7.45% predicts — the
-- app was understating the tax on every restaurant receipt by the full 1%,
-- because `uses_food_rate = 0` on Restaurants was read as "the general rate"
-- when what it actually means is only "not the reduced rate".
--
-- # Why a third rate rather than reusing location_category_rate
--
-- Same reasoning as 002. `location_category_rate` (001) expresses a genuinely
-- local exception for one category at one place. This is a *class* of category
-- at *every* location in the county, so putting it there would mean a row per
-- (location x prepared-food category), rewritten whenever either side gains a
-- member. The rule is "this category is prepared food" and "prepared food is
-- taxed at this rate here", which is what these columns say.
--
-- # Why the rate is stored per jurisdiction and not as "+1%"
--
-- Today the state publishes exactly 1.00% above the combined rate in all 389
-- jurisdictions, so a hardcoded increment would be correct this quarter. It is
-- a county *option*, though — a county may drop it or levy less — and the
-- published table already carries the real figure per jurisdiction. Storing
-- what was published means a county changing its mind is a download, exactly
-- as it is for the combined rate.

-- The published prepared-food rate for this jurisdiction: the combined rate
-- with the county's restaurant tax already added, as the state prints it.
--
-- Stored as the whole rate rather than the increment because that is the
-- figure the publication gives, and deriving the increment here would mean
-- this column disagreed with the source the day a county changed its combined
-- rate and its restaurant tax in the same quarter.
--
-- NULL where no prepared-food rate is published, which must read as "tax
-- prepared food like everything else" and never as "prepared food is free".
ALTER TABLE tax_area ADD COLUMN restaurant_rate_ppm INTEGER
    CHECK (restaurant_rate_ppm IS NULL
           OR restaurant_rate_ppm BETWEEN 0 AND 1000000);

-- The same figure copied onto a location, mirroring `food_rate_ppm` from 002.
-- NULL means this place has no elevated prepared-food rate.
ALTER TABLE location ADD COLUMN restaurant_rate_ppm INTEGER
    CHECK (restaurant_rate_ppm IS NULL
           OR restaurant_rate_ppm BETWEEN 0 AND 1000000);

-- Whether this category is prepared food. `Restaurants` is the only seeded
-- category that sets it; Groceries emphatically does not, since that is the
-- reduced rate from 002 and the two are mutually exclusive by construction in
-- `domain::tax::RateClass`.
--
-- Defaulted to 0 so no existing category changes meaning on upgrade; the seed
-- turns it on for the categories that need it.
ALTER TABLE category ADD COLUMN uses_restaurant_rate INTEGER NOT NULL DEFAULT 0
    CHECK (uses_restaurant_rate IN (0, 1));
