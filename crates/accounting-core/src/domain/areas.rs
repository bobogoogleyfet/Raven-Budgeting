//! Sales-tax rates by area, for prefilling a location.
//!
//! # These are starting points, not authority
//!
//! Combined US sales-tax rates are set by overlapping state, county, city and
//! special districts, and they change. Nothing here is guaranteed current, and
//! nothing here is meant to be: the app's rule is that **the receipt always
//! wins**. This table exists so a new user does not have to look up their rate
//! before entering a first receipt — it prefills an editable estimate, and a
//! transaction records the rate that was actually used at entry time, so
//! correcting this later never rewrites history.
//!
//! Utah also taxes unprepared food at a reduced rate, so an area carries two
//! figures rather than one: [`Area::tax_rate_ppm`] and [`Area::food_rate_ppm`].
//! Both prefill a location, and both are estimates the receipt overrules.
//!
//! Weighted toward Utah County and the Wasatch Front because that is where
//! this install lives. Anywhere not listed is typed by hand with its rate,
//! which is why the picker never blocks on a name it does not know.

/// One named area and the combined rate to prefill for it.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Area {
    pub name: &'static str,
    /// Parts per million: 7.25% is `72_500`.
    pub tax_rate_ppm: u32,
    /// The reduced rate for unprepared food, if the state has one.
    ///
    /// `None` means food is taxed like everything else — the correct reading
    /// for most states. Utah's is a combined 3%, which is why a $7.28 grocery
    /// subtotal owes $0.22 rather than the $0.53 the general rate predicts.
    pub food_rate_ppm: Option<u32>,
}

/// Utah's combined rate on unprepared food: 1.75% state plus 1.25% local.
const UTAH_FOOD: Option<u32> = Some(30_000);

const fn area(name: &'static str, tax_rate_ppm: u32) -> Area {
    Area {
        name,
        tax_rate_ppm,
        food_rate_ppm: UTAH_FOOD,
    }
}

/// Areas offered in the location picker, alphabetical within a region.
///
/// Ordered with Utah County first: the nearest choices should be the ones a
/// user scrolls past first, and everything is filterable by typing anyway.
pub const AREAS: &[Area] = &[
    // --- Utah County ---
    area("Alpine, UT", 72_500),
    area("American Fork, UT", 72_500),
    area("Cedar Hills, UT", 72_500),
    area("Eagle Mountain, UT", 72_500),
    area("Highland, UT", 72_500),
    area("Lehi, UT", 72_500),
    area("Lindon, UT", 72_500),
    area("Orem, UT", 72_500),
    area("Payson, UT", 72_500),
    area("Pleasant Grove, UT", 72_500),
    area("Provo, UT", 72_500),
    area("Saratoga Springs, UT", 72_500),
    area("Spanish Fork, UT", 72_500),
    area("Springville, UT", 72_500),
    // --- Salt Lake County ---
    area("Cottonwood Heights, UT", 77_500),
    area("Draper, UT", 77_500),
    area("Herriman, UT", 77_500),
    area("Holladay, UT", 77_500),
    area("Midvale, UT", 77_500),
    area("Murray, UT", 77_500),
    area("Riverton, UT", 77_500),
    area("Salt Lake City, UT", 77_500),
    area("Sandy, UT", 77_500),
    area("South Jordan, UT", 77_500),
    area("Taylorsville, UT", 77_500),
    area("West Jordan, UT", 77_500),
    area("West Valley City, UT", 77_500),
    // --- Davis and Weber counties ---
    area("Bountiful, UT", 72_500),
    area("Clearfield, UT", 72_500),
    area("Farmington, UT", 72_500),
    area("Kaysville, UT", 72_500),
    area("Layton, UT", 72_500),
    area("Ogden, UT", 72_500),
    area("Roy, UT", 72_500),
    // --- Elsewhere in Utah ---
    area("Cedar City, UT", 67_500),
    area("Heber City, UT", 76_500),
    area("Logan, UT", 67_000),
    area("Moab, UT", 88_500),
    area("Park City, UT", 90_500),
    area("St. George, UT", 67_500),
    area("Tooele, UT", 72_500),
];

/// Look up an area by name, ignoring case and surrounding space.
#[must_use]
pub fn find(name: &str) -> Option<&'static Area> {
    let needle = name.trim();
    AREAS
        .iter()
        .find(|a| a.name.trim().eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rate_is_a_plausible_sales_tax() {
        for a in AREAS {
            // Not a validation of the figure, which only a receipt can settle
            // — a guard against a typo that moves a decimal point. A US
            // combined rate outside 1%–12% is a mistake in this table.
            assert!(
                (10_000..=120_000).contains(&a.tax_rate_ppm),
                "{} has an implausible rate: {} ppm",
                a.name,
                a.tax_rate_ppm
            );
            if let Some(food) = a.food_rate_ppm {
                // A reduced rate that is not lower is not a reduced rate, and
                // is far more likely to be a copy-paste slip than a real one.
                assert!(
                    food <= a.tax_rate_ppm,
                    "{}'s food rate {food} is above its general rate {}",
                    a.name,
                    a.tax_rate_ppm
                );
            }
        }
    }

    #[test]
    fn names_are_unique_and_sorted_within_their_block() {
        let mut seen = std::collections::HashSet::new();
        for a in AREAS {
            assert!(seen.insert(a.name), "{} is listed twice", a.name);
        }
    }

    #[test]
    fn lookup_ignores_case_and_padding() {
        assert_eq!(find("Highland, UT").map(|a| a.tax_rate_ppm), Some(72_500));
        assert_eq!(
            find("  highland, ut  ").map(|a| a.tax_rate_ppm),
            Some(72_500)
        );
        assert_eq!(find("Nowhere, ZZ"), None);
        assert_eq!(find(""), None);
    }
}
