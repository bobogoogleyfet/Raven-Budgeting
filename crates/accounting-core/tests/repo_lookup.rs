//! Lookup-side repository tests: categories, locations, merchants.
//!
//! These three repositories are the entry screen's whole vocabulary. A user
//! types "groc" and a store name; everything else — the tax rate, the default
//! category, whether to itemize — is *recalled* rather than asked for. So the
//! behaviour under test here is mostly about forgiveness (case, prefixes,
//! stray whitespace) and about memory that does not decay: the store-memory
//! bugs that matter are the ones where a later, less specific entry quietly
//! erases what the app had already learned.
//!
//! Where an error is the user's only route to recovery — an ambiguous category
//! name, a name that does not exist — the *message* is asserted, not just the
//! variant. "invalid input" with no candidates listed is a dead end.

mod common;

use accounting_core::repo::{categories, locations, merchants};
use accounting_core::{CategoryGroupId, CategoryId, CoreError, Db, DetailLevel};
use common::{assert_clean, count, raw, seeded_db};

/// The id of a seeded category group, by name.
fn group(db: &Db, name: &str) -> CategoryGroupId {
    categories::list_groups(db)
        .expect("list_groups")
        .into_iter()
        .find(|g| g.name == name)
        .unwrap_or_else(|| panic!("the seed taxonomy must contain a `{name}` group"))
        .id
}

/// Create a category and read it back through `resolve`.
///
/// Deliberately does not use `create`'s return value: what matters to every
/// test below is that the row is subsequently *findable*, which is a stronger
/// statement and independent of whether `create` hands back a row or an id.
fn create_category(db: &Db, group_name: &str, name: &str) -> CategoryId {
    let g = group(db, group_name);
    categories::create(db, g, name).unwrap_or_else(|e| panic!("create {group_name}/{name}: {e}"));
    categories::resolve(db, &format!("{group_name}/{name}"))
        .unwrap_or_else(|e| panic!("resolve the category just created: {e}"))
        .id
}

// ---------------------------------------------------------------------------
// categories::resolve — the three accepted forms
// ---------------------------------------------------------------------------

#[test]
fn resolve_finds_a_category_by_its_exact_name() {
    let db = seeded_db();

    let found = categories::resolve(&db, "Groceries").expect("resolve `Groceries`");
    assert_eq!(found.name, "Groceries");
    assert_eq!(found.group_name, "Frequent");

    // Case is the user's business, not ours: what is printed on the receipt
    // rarely matches the capitalisation in the taxonomy.
    let shouted = categories::resolve(&db, "GROCERIES").expect("resolve `GROCERIES`");
    assert_eq!(shouted.id, found.id);
}

#[test]
fn resolve_finds_a_category_by_group_and_name() {
    let db = seeded_db();

    let qualified = categories::resolve(&db, "Frequent/Groceries").expect("resolve qualified");
    assert_eq!(
        qualified.id,
        categories::resolve(&db, "Groceries").unwrap().id
    );

    let lowered = categories::resolve(&db, "frequent/groceries").expect("resolve lowercased");
    assert_eq!(lowered.id, qualified.id);

    // The qualified form is the disambiguation route the error message points
    // users at, so it must work for a name that contains punctuation too.
    let insurance = categories::resolve(&db, "Bills/Insurance (Auto)").expect("resolve insurance");
    assert_eq!(insurance.name, "Insurance (Auto)");
    assert_eq!(insurance.group_name, "Bills");
}

#[test]
fn resolve_finds_a_category_by_a_unique_prefix() {
    let db = seeded_db();

    // "Groc" is a prefix of "Groceries" and of nothing else in the seed.
    assert_eq!(
        categories::resolve(&db, "Groc").expect("prefix").name,
        "Groceries"
    );
    // Prefix matching is case-insensitive like the other two forms.
    assert_eq!(
        categories::resolve(&db, "emerg").expect("prefix").name,
        "Emergency Fund"
    );
}

#[test]
fn an_ambiguous_bare_name_reports_every_candidate() {
    // The single most important error message in the CLI. A user who types an
    // ambiguous name has to be told what to type instead; "invalid input:
    // ambiguous" leaves them guessing at group names they may never have seen.
    let db = seeded_db();
    create_category(&db, "Bills", "Utilities");
    create_category(&db, "Quality of Life", "Utilities");

    let err =
        categories::resolve(&db, "Utilities").expect_err("an ambiguous name must not resolve");
    assert!(
        matches!(err, CoreError::Invalid(_)),
        "an ambiguous name is bad input, not a missing row; got {err:?}"
    );

    let msg = err.to_string();
    assert!(
        msg.contains("Utilities"),
        "the message must name the needle; got: {msg}"
    );
    let bills_at = msg
        .find("Bills/Utilities")
        .unwrap_or_else(|| panic!("the message must list `Bills/Utilities`; got: {msg}"));
    let qol_at = msg
        .find("Quality of Life/Utilities")
        .unwrap_or_else(|| panic!("the message must list `Quality of Life/Utilities`; got: {msg}"));
    assert!(
        bills_at < qol_at,
        "candidates must be listed in sorted order so the list is stable between runs; got: {msg}"
    );

    assert_clean(&db);
}

#[test]
fn an_unknown_category_name_is_not_found_and_says_what_was_looked_up() {
    let db = seeded_db();

    let err = categories::resolve(&db, "Zorblatt").expect_err("no such category");
    assert!(
        matches!(err, CoreError::NotFound(_)),
        "a name nobody has ever used is missing, not invalid; got {err:?}"
    );
    assert!(
        err.to_string().contains("Zorblatt"),
        "the message must echo the needle so the user can see their typo; got: {err}"
    );

    // The same story through the by-id path.
    let err = categories::get(&db, CategoryId::new(999_999)).expect_err("no such id");
    assert!(matches!(err, CoreError::NotFound(_)), "got {err:?}");
}

#[test]
fn find_by_name_declines_to_guess_when_a_name_is_ambiguous() {
    // `find_by_name` is the non-throwing lookup used where a miss is ordinary.
    // An ambiguous name must come back as `None` rather than as whichever row
    // the query planner happened to return first — silently picking a category
    // is how spend lands in the wrong envelope.
    let db = seeded_db();
    create_category(&db, "Bills", "Utilities");
    create_category(&db, "Quality of Life", "Utilities");

    assert!(
        categories::find_by_name(&db, "Utilities")
            .expect("find_by_name")
            .is_none(),
        "an ambiguous bare name must resolve to nothing at all"
    );
    assert_eq!(
        categories::find_by_name(&db, "groceries")
            .expect("find_by_name")
            .map(|c| c.name),
        Some("Groceries".to_string())
    );
    assert!(
        categories::find_by_name(&db, "Zorblatt")
            .expect("find_by_name")
            .is_none()
    );

    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Archiving
// ---------------------------------------------------------------------------

#[test]
fn archiving_a_category_hides_it_everywhere_and_frees_its_name() {
    let db = seeded_db();
    let first = create_category(&db, "Bills", "Storage Unit");
    assert!(
        categories::list(&db).unwrap().iter().any(|c| c.id == first),
        "a freshly created category must be offered in the picker"
    );

    categories::archive(&db, first).expect("archive");

    assert!(
        !categories::list(&db).unwrap().iter().any(|c| c.id == first),
        "an archived category must not be offered in the picker"
    );
    assert!(
        matches!(
            categories::resolve(&db, "Storage Unit").expect_err("archived"),
            CoreError::NotFound(_)
        ),
        "an archived category must not be resolvable by name either"
    );

    // ux_category_name is a *partial* unique index (WHERE archived_at IS NULL),
    // so archiving releases the name rather than burning it forever. The row
    // itself survives, because historical splits must keep resolving a name.
    let second = create_category(&db, "Bills", "Storage Unit");
    assert_ne!(
        second, first,
        "recreating an archived name must make a new row, not resurrect the old one"
    );
    assert_eq!(
        categories::resolve(&db, "Storage Unit")
            .expect("resolve")
            .id,
        second
    );
    assert_eq!(
        count(
            raw(&db),
            "SELECT COUNT(*) FROM category WHERE name = 'Storage Unit'"
        ),
        2,
        "the archived row must still exist so old transactions keep their name"
    );

    assert_clean(&db);
}

#[test]
fn list_is_ordered_by_group_then_category_then_name() {
    let db = seeded_db();
    let group_order: std::collections::HashMap<CategoryGroupId, i64> = categories::list_groups(&db)
        .expect("list_groups")
        .into_iter()
        .map(|g| (g.id, g.sort_order))
        .collect();

    let listed = categories::list(&db).expect("list");
    assert!(!listed.is_empty(), "the seed installs a taxonomy");

    let keys: Vec<(i64, i64, String)> = listed
        .iter()
        .map(|c| {
            let g = *group_order.get(&c.group_id).unwrap_or_else(|| {
                panic!("category {} names a group that list_groups omits", c.name)
            });
            (g, c.sort_order, c.name.clone())
        })
        .collect();

    // Asserting the ordering *property* rather than a literal list, so adding
    // a seed category does not break a test that is about sorting.
    assert!(
        keys.windows(2).all(|w| w[0] <= w[1]),
        "categories must come back grouped and sorted; got {keys:?}"
    );
}

#[test]
fn the_uncategorized_category_is_found_by_key_not_by_name() {
    // Quick-add depends on this row existing; the user is allowed to rename it
    // to "Inbox", and doing so must not strand every uncategorised receipt.
    let db = seeded_db();
    let before = categories::uncategorized(&db).expect("uncategorized");
    assert_eq!(before.system_key.as_deref(), Some("uncategorized"));

    raw(&db)
        .execute(
            "UPDATE category SET name = 'Inbox' WHERE system_key = 'uncategorized'",
            [],
        )
        .expect("rename");

    let after = categories::uncategorized(&db).expect("uncategorized after rename");
    assert_eq!(after.id, before.id);
    assert_eq!(after.name, "Inbox");

    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// locations
// ---------------------------------------------------------------------------

#[test]
fn get_or_create_never_overwrites_an_existing_locations_rate() {
    let db = seeded_db();

    // The seed installs Home at 0 ppm deliberately: a wrong prefilled rate is
    // worse than none, because it produces plausible tax figures that quietly
    // disagree with the receipt. A later `get_or_create` carrying a guess must
    // not smuggle that guess in.
    locations::get_or_create(&db, "Home", 83_750).expect("get_or_create Home");
    let home = locations::find_by_name(&db, "Home")
        .expect("find_by_name")
        .expect("the seed installs Home");
    assert_eq!(
        home.tax_rate_ppm, 0,
        "an existing rate must survive get_or_create"
    );
    assert_eq!(
        count(
            raw(&db),
            "SELECT COUNT(*) FROM location WHERE name = 'Home'"
        ),
        1,
        "get_or_create must reuse the existing row rather than adding a second"
    );

    // A genuinely new location does take the rate it is created with.
    locations::get_or_create(&db, "Reno", 82_650).expect("create Reno");
    let reno = locations::find_by_name(&db, "Reno").unwrap().expect("Reno");
    assert_eq!(reno.tax_rate_ppm, 82_650);

    // And `set_rate` is the explicit, intentional way to change one.
    locations::set_rate(&db, reno.id, 91_250).expect("set_rate");
    assert_eq!(
        locations::get(&db, reno.id).expect("get").tax_rate_ppm,
        91_250
    );

    assert_clean(&db);
}

#[test]
fn set_home_moves_the_flag_instead_of_adding_a_second_home() {
    let db = seeded_db();
    let previous = locations::home(&db)
        .expect("home")
        .expect("the seed installs a home location");
    assert_eq!(previous.name, "Home");

    locations::get_or_create(&db, "Austin, TX", 82_500).expect("create Austin");
    let austin = locations::find_by_name(&db, "Austin, TX")
        .unwrap()
        .expect("Austin");
    locations::set_home(&db, austin.id).expect("set_home");

    assert_eq!(
        locations::home(&db).expect("home").map(|l| l.id),
        Some(austin.id)
    );
    assert!(
        !locations::get(&db, previous.id).expect("get").is_home,
        "the previous home must have been demoted, not merely outranked"
    );
    // `ux_location_home` is a partial unique index allowing exactly one home,
    // so an implementation that inserted before clearing would fail outright;
    // this pins that the clear happens in the same transaction.
    assert_eq!(
        count(raw(&db), "SELECT COUNT(*) FROM location WHERE is_home = 1"),
        1,
        "there must be exactly one home location at all times"
    );

    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// merchants — the store memory
// ---------------------------------------------------------------------------

#[test]
fn get_or_create_trims_a_merchant_name_and_refuses_an_empty_one() {
    let db = seeded_db();

    merchants::get_or_create(&db, "  Costco  ").expect("create with padding");
    let costco = merchants::find_by_name(&db, "Costco")
        .expect("find_by_name")
        .expect("the padded name must have been stored trimmed");
    assert_eq!(costco.name, "Costco");

    // The trim has to happen *before* the lookup, or every stray space in a
    // pasted receipt spawns a second merchant with the same visible name.
    merchants::get_or_create(&db, "Costco").expect("second get_or_create");
    merchants::get_or_create(&db, " Costco").expect("third get_or_create");
    // Counted by name rather than over the whole table: the seed installs
    // suggested stores (Costco among them), so this also pins that
    // get_or_create reuses the seeded row instead of shadowing it with a
    // second one the user would then see twice in the picker.
    assert_eq!(
        count(
            raw(&db),
            "SELECT COUNT(*) FROM merchant WHERE name = 'Costco'"
        ),
        1,
        "the same store, however it was typed, must be one row"
    );

    let err = merchants::get_or_create(&db, "   ").expect_err("whitespace-only name");
    assert!(
        matches!(err, CoreError::Invalid(_)),
        "an empty merchant name is bad input, not a new merchant; got {err:?}"
    );

    assert_clean(&db);
}

#[test]
fn remembering_without_a_category_leaves_the_previous_one_intact() {
    // The store-memory regression that matters most. `remember` is called on
    // every save, including saves where the user split across three categories
    // and so passed no single default. If `None` meant "forget", one such
    // receipt would erase a default the store had recalled correctly for
    // months, and the entry screen would silently go back to asking.
    let db = seeded_db();
    merchants::get_or_create(&db, "Costco").expect("create merchant");
    let costco = merchants::find_by_name(&db, "Costco")
        .unwrap()
        .expect("Costco");
    let groceries = categories::resolve(&db, "Groceries").expect("resolve").id;
    locations::get_or_create(&db, "Costco Sunnyvale", 91_250).expect("create location");
    let store = locations::find_by_name(&db, "Costco Sunnyvale")
        .unwrap()
        .expect("store");

    merchants::remember(
        &db,
        costco.id,
        Some(groceries),
        Some(store.id),
        DetailLevel::Itemized,
    )
    .expect("remember");

    merchants::remember(&db, costco.id, None, None, DetailLevel::Grouped).expect("remember again");

    let after = merchants::get(&db, costco.id).expect("get");
    assert_eq!(
        after.default_category_id,
        Some(groceries),
        "`None` means the caller had nothing to say, not that the memory should be cleared"
    );
    assert_eq!(after.default_location_id, Some(store.id));
    // The detail level is the one field with no "unknown" value, so it always
    // takes the level the receipt was actually entered at.
    assert_eq!(after.default_detail_level, DetailLevel::Grouped);

    assert_clean(&db);
}

#[test]
fn recent_merchants_are_ordered_by_last_use_with_never_used_ones_last() {
    let db = seeded_db();
    for name in ["Alpha", "Beta", "Never Used"] {
        merchants::get_or_create(&db, name).unwrap_or_else(|e| panic!("create {name}: {e}"));
    }
    let id_of = |name: &str| {
        merchants::find_by_name(&db, name)
            .expect("find_by_name")
            .unwrap_or_else(|| panic!("{name} must exist"))
            .id
    };

    merchants::touch(&db, id_of("Alpha"), "2026-01-05T10:00:00.000Z").expect("touch Alpha");
    merchants::touch(&db, id_of("Beta"), "2026-03-09T10:00:00.000Z").expect("touch Beta");

    // A limit past the seeded suggestions, so the never-used store under test
    // is actually in the window being asserted about.
    let names: Vec<String> = merchants::recent(&db, 500)
        .expect("recent")
        .into_iter()
        .map(|m| m.name)
        .collect();
    let at = |name: &str| {
        names
            .iter()
            .position(|n| n == name)
            .unwrap_or_else(|| panic!("{name} missing from {names:?}"))
    };
    assert!(
        at("Beta") < at("Alpha"),
        "the most recently used store must come first; got {names:?}"
    );
    assert!(
        at("Alpha") < at("Never Used"),
        "a store that has never been used has nothing to recommend it and must sort last; \
         got {names:?}"
    );

    // The limit is a limit, not a suggestion: the entry screen renders these
    // as chips and cannot grow a scrollbar.
    let top = merchants::recent(&db, 1).expect("recent");
    assert!(top.len() <= 1, "recent(1) returned {} rows", top.len());
    assert_eq!(top.first().map(|m| m.name.as_str()), Some("Beta"));

    assert_clean(&db);
}

#[test]
fn touch_stores_exactly_the_timestamp_it_is_given() {
    // The core never reads the clock — every time-dependent function is handed
    // its "now". If `touch` reached for `strftime('now')` instead, this suite
    // would still pass everywhere else and every test involving ordering by
    // last use would become time-dependent.
    let db = seeded_db();
    merchants::get_or_create(&db, "Costco").expect("create");
    let costco = merchants::find_by_name(&db, "Costco")
        .unwrap()
        .expect("Costco");

    merchants::touch(&db, costco.id, "2026-08-13T12:34:56.789Z").expect("touch");

    assert_eq!(
        merchants::get(&db, costco.id).expect("get").last_used_at,
        Some("2026-08-13T12:34:56.789Z".to_string())
    );

    assert_clean(&db);
}
