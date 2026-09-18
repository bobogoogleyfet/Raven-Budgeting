# Porting the desktop app to Flutter

## Why

The app is meant to reach Android and iOS. The desktop frontend was
TypeScript in a Tauri webview, which does not go there — Tauri's mobile story
is young, and a webview UI on a phone is a webview UI on a phone. Flutter
renders natively on all five targets from one codebase, and Material Design 3
gives the interface a specification to follow rather than a stylesheet someone
has to keep inventing.

Material 3 is followed to the letter, not approximately: components and tokens
as specified, `ColorScheme.fromSeed` for the palette, and spacing, shape,
elevation and type taken from the token set rather than picked by eye. Where
the spec and a habit carried over from the TypeScript UI disagree, the spec
wins and the behaviour rule being preserved gets written down.

The core does not change. `accounting-core` is 14,258 lines of domain, storage
and export with four boundary rules — no UI, no path discovery, fully
synchronous, all money math inside — written precisely so that a second
frontend is a rendering exercise. This port is the first real test of that
claim, and the spike below suggests it holds.

## What is already proven

Not researched — built and run, on this machine, 2026-08-14.

| Question | Answer | Evidence |
|---|---|---|
| Does bundled SQLite work inside a `cdylib`? | Yes | A `cdylib` linking `accounting-core`, loaded with `ctypes`, opened a database, migrated it and seeded 24 categories |
| Does Flutter build for Linux desktop here? | Yes | `flutter build linux` on a scratch project |
| Does the core work from Dart? | Yes | `flutter test integration_test/` — 2 tests, both passing |
| Is the money right across the bridge? | Yes | $7.28 of groceries → $0.22 tax → **$7.50**, `appliedRatePpm == 30000` |

Versions: Flutter 3.47.0 (upgraded from 3.24.5), Dart 3.10, `flutter_rust_bridge`
2.12.0, codegen `flutter_rust_bridge_codegen 2.12.0`.

Rechecked 2026-08-20: Flutter is still 3.47.0 and frb 2.12.0 is still the
latest stable (2.13.0 is in beta), but Dart is **3.13.0**, not 3.10 — newer,
and inside frb's `>=3.4.0 <4.0.0` constraint, so nothing had to move.

### Type mapping, observed rather than assumed

Generated from the real core, not from a tutorial:

| Rust | Dart |
|---|---|
| `i64` **in a struct field** | `PlatformInt64`, which is `typedef = int` on native |
| `i64` **in a bare `Vec`** | `BigInt` — see below |
| `Option<u32>` | `int?` |
| `Vec<String>` | `List<String>` |
| `Vec<Struct>` | `List<Struct>` |
| `Result<T, String>` | `Future<T>` that throws |
| doc comments | carried through to the Dart |

**Money goes in a struct field, never a bare `Vec<i64>`.** The bridge ships its
own `Int64List extends TypedList<BigInt, int>`, so indexing a returned
`Vec<i64>` yields `BigInt` — on native, and with web generation disabled. The
values are exact either way (`i64::MAX`, `i64::MIN` and 2^53+1 all survive), so
this is an ergonomic trap rather than a correctness one: every amount would
need unwrapping, and one missed conversion is a type error rather than silent
corruption. The existing DTOs already return lists of structs, so the rule
costs nothing — it just has to be written down before someone designs an
endpoint around `Vec<i64>`.

This was worth checking rather than assuming: research reported "i64
round-trips exactly", which is true of the value and silent about the type.

Every generated function is a `Future`, so a blocking SQLite call runs off the
UI isolate without being marked. That matches the core's design: it is
synchronous because SQLite is a file, and the bridge is what makes that
acceptable to a UI.

## Shape

```
crates/accounting-core/     unchanged — the product
crates/accounting-cli/      unchanged — pacc
app/                        the Flutter application
  rust/                     the bridge crate: crate-type cdylib+staticlib
    src/api/                the only Rust the codegen reads
  lib/src/rust/             generated Dart — committed, never edited
  lib/                      the Flutter app
  test/                     pure-logic tests, ported from ui/src/*.test.ts
  integration_test/         tests that call the real core
```

The bridge crate was built as a *third* frontend beside the CLI and Tauri, not
a replacement layer under them. It re-exposed the same surface the Tauri shell
did — 19 commands — and holds no logic, for the same reason that shell held
none: logic there would have to be written again for every platform. With Tauri
retired it is one of two frontends over the core, on the same terms.

## Phases

Each phase ends with something runnable. The Tauri app stayed usable throughout,
so there was never a day without a working expense tracker.

1. **Foundation — built 2026-08-20.** `app/` scaffolded, bridge crate wired to
   `accounting-core`, database path resolution (the CLI and Tauri already share
   `pacc.db`; Flutter must too), M3 theme, adaptive navigation shell. Ends with
   an app that opens the real database and lists categories.

   Run against the real `~/.local/share/personal-accounting/pacc.db`: it opens
   the same file the CLI and Tauri do, and lists the 24 seeded categories under
   their group headings.
2. **Money and dates — built 2026-08-20.** Port `money.ts` and the date logic
   to Dart with their tests, including the round-trip of every cent from 0 to
   20,000. Nothing else is built on floats, so this comes first.

   Dart enforces the rule better than TypeScript could: `Cents` is an extension
   type that does not implement `int`, so `a - b` on two of them does not
   compile at all, where the TS brand could only make the *result* unusable.
   Two cases were added that the original does not have — the thousands
   separator and a negative, neither of which its loop reaches.
3. **Expenses list — built 2026-08-20.** Table, sort caret, spreadsheet filter
   menus, delete confirmation. The most self-contained screen and the best
   shakedown for the table story.

   A table where there is width and a list of tiles on a phone, with one set of
   controls above both: the rules the filter menus carry are fiddly enough
   written once. One addition to what the TypeScript does — the list says when
   it is showing only the most recent 200, because a list that silently stops
   is one you trust wrongly.
4. **Entry form — built 2026-08-20.** The hardest screen: repeating category
   rows, the pickers, the review dialog with its editable tax. Every behaviour
   rule in the inventory applies here.

   Two corrections to the plan, found by reading the original rather than
   trusting this document. `taxEdited` lives in the review dialog, not the
   form. And there is no shape decision to port: the add screen calls
   `preview_subtotals` / `save_subtotals` unconditionally, while
   `preview_transaction` / `save_transaction` have no caller in the UI at all —
   they are the total-first path `pacc add` uses.

   One deliberate deviation: Save stays disabled until a preview says the
   receipt can be saved. The TypeScript leaves it enabled on an untouched form,
   which opens a dialog with no rows and its own Save disabled.

   The first-run gate arrived with phase 5.
5. **Inbox, Budget, Settings, first run — built 2026-08-20.** Budgets can also
   be set from the app rather than only from `pacc budget set`: one dialog,
   reached from the Budget screen and from the overview's empty state.
6. **Android.** NDK, `cargo-ndk`, Rust targets, and the first real test of
   bundled SQLite cross-compiling. Deliberately last: nothing before it depends
   on Android, and everything before it de-risks it.
7. **Retire Tauri — done 2026-08-20.** `crates/accounting-tauri` and `ui/`
   deleted, and the stale-bundle build guard with them.

## Navigation

Five destinations — Home, Expenses, Inbox, Budget, Settings — and entering an
expense is **not** one of them. It is a task rather than a place, so it lives
on the button that floats over every screen: the rail's leading slot where
there is one, a floating button where there is not. The app therefore opens one
tap away from recording something, which is the point.

Home is the budget at a glance: a ring of per-category meters with the month's
spending against the month's budget in the hole, and the three most recent
expenses under it. Past 1200dp the recent expenses sit beside the budget rather
than below it.

The ring is not a pie. A pie encodes one value per slice as an angle; each of
these arcs encodes two — its length is that category's share of the month's
budget, and its fill is how much of that category has gone. Over budget fills
the arc and turns it red, because there is no 110% of an arc.

Each segment carries its own hue so the legend beside it can name them, and
each draws its track in that hue at low opacity with its spending over the top
— so the arc's *length* is readable before a penny is spent. Over budget takes
the error colour and fills, whatever hue the category had: red means one thing
here.

The palette is seven slots in a fixed order (`widgets/chart_palette.dart`),
validated rather than picked: worst adjacent CVD separation ΔE 9.1 light and
8.4 dark against a target of 8, worst normal-vision ΔE 19.6 and 19.3 against a
floor of 15. Three light slots fall under 3:1 against the surface, which the
validator flags as needing relief; the legend is that relief, and is why
identity never rests on colour alone.

Seven and not eight, because the reference palette's eighth slot is a red and
red is reserved for over budget. Past the seventh, categories fold into one
grey arc — a generated eighth hue is indistinguishable from an existing one
under CVD — except when the tail is a single category, which keeps its name.

Selecting a category steps its arc onto a wider circle — the same angles, so it
still sits over its own share — and lets the rest recede. Growing the stroke
instead moved the arc's own edges, which made choosing one look like the data
had changed.

The legend names the seven the ring draws, then one row for the tail, then a
link to the Budget screen. It deliberately does not list every budget: with a
full chart of accounts budgeted that is twenty rows under a summary, which is
a worse copy of a screen that already exists.

The ring is drawn, not drawn *on*: arcs painted as paths, so it is
resolution-independent and stays crisp at any device pixel ratio — checked at
2x. Its jaggedness was never resolution; it was per-segment tracks turning
every gap into a notch in the ring's outline.

This settled after one wrong turn worth recording: entering an expense was
briefly the home surface itself, with no destination selected. That works on a
rail and a drawer, which both accept a null selection, and does not work on a
`NavigationBar`, which asserts a valid index. Making the form a task and Home a
place removed the special case rather than papering over it.

## Known traps carried over deliberately

Two behaviours were copied rather than fixed, because changing them is a
product decision and not a port decision. Both are recorded here so the next
person knows they were seen.

- **A genuinely 0% area is asked forever.** The gate is "no home, or a home
  whose rate is 0", which is what the seed installs. Someone in a place with no
  sales tax types 0, and is asked again on every launch. Fixing it means
  recording "this was answered" separately from the rate.
- **The area picker matches the published name exactly, and the core does
  not.** Type "Provo, UT" and the picker says it is not in Utah's list, so the
  rate gets typed by hand and the location stops following the quarterly
  publication — silently, and forever. `tax_areas::match_key` strips a `, UT`
  suffix; the picker does not.

Closed since: the app fetches the quarterly table itself. `api/rates.rs` does
the one thing the core is not allowed to do — pull the bytes — and everything
worth deciding stays in `service::rates`. It runs unawaited at launch, drops
the database lock before the network call, and records the check before the
attempt so a machine with no route out tries once a day rather than on every
launch. Settings also has a "Check for new rates" button, which the TypeScript
has no equivalent of: the provenance line makes staleness visible, and that
makes it actionable.

## What must survive the port

The behaviour inventory is the specification. The rules most likely to be lost,
because they look like implementation detail and are not:

- **Rows are never re-rendered while the user is in them.** Re-rendering
  replaces the input mid-click, focus goes to a detached element, and the field
  needs clicking twice. Fixed once already this session.
- **`taxEdited` per row.** Only figures the user touched are sent as overrides,
  or every save claims to have been read off the receipt and `taxIsEstimated`
  stops meaning anything.
- **Everything ticked is `null`, not a list of every id.** They return the same
  rows today and different rows tomorrow.
- **`today()` from local getters, never `toISOString()`.** At 8pm in Utah that
  files a receipt into tomorrow, and at a month boundary into the wrong budget.
- **The rows are keyed by identity.** The "never re-render a row the user is
  in" rule is three in-place mutations in the TypeScript, avoiding a rebuild
  that would take the caret with it. In Flutter it is a key on the row widget,
  which is the same rule expressed in the framework's own terms.
- **A total is arithmetic, so the core owns it.** `service::budgets::
  month_totals` sums the month's contributions and spending, rather than a
  frontend adding the rows up. Both figures are the month's, not the cycle's: a
  $780 insurance bill in a month's budget total would swamp every other
  envelope and answer a question nobody asked.
- **A DTO may need a field the Tauri shell never exposed.** `BudgetRowDto`
  there drops `SpendStatus::remaining`, because that screen shows funding and
  overspending and never "left". The home screen answers "how much is left",
  so the bridge exposes the core's own figure rather than subtracting spent
  from budgeted in Dart.
- **Money never computed in the UI.** TypeScript enforced this with a branded
  `Cents` type — arithmetic compiles but the result cannot be sent back. Dart
  has no such brand by default: use an extension type, or the rule evaporates
  silently.
- **A row with an unparseable amount is omitted, never sent as zero.**
- **Hide-all is answered in the UI**, because the core reads an empty filter set
  as "no filter".

## Operating the bridge

Rules that come from how `flutter_rust_bridge` 2.12.0 actually behaves, not
from its documentation. Each one is cheap to follow now and expensive to
discover later.

- **Install a logger ourselves.** `setup_default_user_utils()` calls
  `setup_log_to_console`, which has arms for Android, iOS/macOS and wasm and
  ends in `// TODO add more platforms` — verified in the vendored 2.12.0
  source. On Linux and Windows it is a no-op, so every `log::` call from Rust
  vanishes on the platform we develop on. Add `env_logger` in the bridge crate.
- **Make `#[frb(init)]` idempotent.** A Flutter hot restart resets the Dart
  isolate but not the loaded library, so init runs again against Rust globals
  that were never cleared. Anything that must happen once needs to say so.
- **Never `panic = "abort"`.** The bridge turns a Rust panic into a Dart
  `PanicException` by catching the unwind; under `abort` the process dies at
  the panic site instead, and the generated FFI decoders contain their own
  `unwrap()`s.
- **Set `strip = "symbols"` in `[profile.release]`.** The build tooling does
  not strip and the template ships no release profile at all, so we inherit
  Cargo's defaults.
- **Do not reflexively `flutter clean`.** The Rust target directory lives
  inside the trees it deletes, so a clean means a cold rebuild of SQLite.
- **`Vec<i64>` arguments arrive as `Int64List`** — the bridge's own class of
  that name, not `dart:typed_data`'s. Mixing them up is a type error that names
  the same type twice and explains nothing. Convert once at the boundary; the
  rule that money stays in a struct field means only ids ever go through it.
- **Name the Rust crate with underscores.** The scaffolder copies
  `--rust-crate-name` verbatim into `rust_builder/pubspec.yaml`, where a
  hyphen is not a legal Dart package name. It fails several steps later as
  `Not a valid package name: "null"`, having already created the project.
- **Pin all four packages to one version** — the Dart package, the Rust crate,
  the codegen and the macros. Minor versions may change the handler ABI, and
  the symptom is a content-hash mismatch at runtime.
- **Keep the core synchronous, and mark every bridged function `async`.**
  Async Rust across this bridge requires `Send` futures, which is what makes
  SQLx and SeaORM painful here; our core is synchronous by rule. But an
  ordinary `fn` is run on the bridge's thread pool, which in a browser is Web
  Workers sharing memory — and that needs cross-origin isolation headers
  GitHub Pages cannot send. So each bridged function is a `pub async fn` whose
  body is the same blocking call it always was: tokio's workers run it on a
  desktop or phone, the page's event loop runs it on the web, and no pool is
  ever made. A test in `app/rust/src/api/mod.rs` fails if an ordinary `fn`
  creeps back in. `#[frb(sync)]` is still fine for something that must answer
  on the UI thread.

## Known constraints

From research, worth knowing before they cost a day:

- **Linux desktop is single-window.** The multi-window issue is the platform's
  most-requested and has been open since 2019.
- **Screen readers do not announce dropdowns on Linux.** The combo box role is
  unimplemented. Do not promise accessibility parity on desktop.
- **Tab traversal does not scroll a lazy `ListView`.** The repeating category
  rows go in a `Column` inside a scroll view.
- **The l10n synthetic package was removed in 3.32.** Every older tutorial shows
  an import that no longer exists.
- **A null `shadowColor` renders no shadow under M3.** "Elevation did nothing"
  is correct behaviour.
- **`NumberFormat` is display-only.** Its parser is unreliable for currency and
  it emits non-breaking spaces that break naive test comparisons — which is why
  parsing stays hand-written, as it already is.
- **Stay on `package:flutter/material.dart`.** Material moves out of the SDK in
  3.50, but migration is one `dart fix` command later, whereas adopting the new
  package now means running a two-day-old compatibility bridge across every
  third-party dependency.
