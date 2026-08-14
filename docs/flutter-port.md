# Porting the desktop app to Flutter

## Why

The app is meant to reach Android and iOS. The current desktop frontend is
TypeScript in a Tauri webview, which does not go there — Tauri's mobile story
is young, and a webview UI on a phone is a webview UI on a phone. Flutter
renders natively on all five targets from one codebase, and Material Design 3
gives the interface a specification to follow rather than a stylesheet someone
has to keep inventing.

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

### Type mapping, observed rather than assumed

Generated from the real core, not from a tutorial:

| Rust | Dart |
|---|---|
| `i64` | `PlatformInt64` (an `int` on native; `BigInt` only on web, which is not a target) |
| `Option<u32>` | `int?` |
| `Vec<String>` | `List<String>` |
| `Vec<Struct>` | `List<Struct>` |
| `Result<T, String>` | `Future<T>` that throws |
| doc comments | carried through to the Dart |

Every generated function is a `Future`, so a blocking SQLite call runs off the
UI isolate without being marked. That matches the core's design: it is
synchronous because SQLite is a file, and the bridge is what makes that
acceptable to a UI.

## Shape

```
crates/accounting-core/     unchanged — the product
crates/accounting-cli/      unchanged — pacc
crates/accounting-tauri/    kept until Flutter reaches parity, then deleted
app/                        the Flutter application
  rust/                     the bridge crate: crate-type cdylib+staticlib
    src/api/                the only Rust the codegen reads
  lib/src/rust/             generated Dart — committed, never edited
  lib/                      the Flutter app
  test/                     pure-logic tests, ported from ui/src/*.test.ts
  integration_test/         tests that call the real core
```

The bridge crate is a *third* frontend beside the CLI and Tauri, not a
replacement layer under them. It re-exposes the same surface the Tauri shell
does — 19 commands — and holds no logic, for the same reason that shell holds
none: logic there would have to be written again for every platform.

## Phases

Each phase ends with something runnable. The Tauri app stays usable throughout,
so there is never a day without a working expense tracker.

1. **Foundation.** `app/` scaffolded, bridge crate wired to `accounting-core`,
   database path resolution (the CLI and Tauri already share `pacc.db`; Flutter
   must too), M3 theme, adaptive navigation shell. Ends with an app that opens
   the real database and lists categories.
2. **Money and dates.** Port `money.ts` and the date logic to Dart with their
   tests — 12 money tests and 8 date tests, including the round-trip of every
   cent from 0 to 20,000. Nothing else is built on floats, so this comes first.
3. **Expenses list.** Table, sort caret, spreadsheet filter menus, delete
   confirmation. The most self-contained screen and the best shakedown for the
   table story.
4. **Entry form.** The hardest screen: repeating category rows, three
   autocompletes, the review dialog with its editable tax. Every behaviour rule
   in the inventory applies here.
5. **Inbox, Budget, Settings, first run.**
6. **Android.** NDK, `cargo-ndk`, Rust targets, and the first real test of
   bundled SQLite cross-compiling. Deliberately last: nothing before it depends
   on Android, and everything before it de-risks it.
7. **Retire Tauri.** Delete `crates/accounting-tauri` and `ui/`, and the
   stale-bundle build guard with them.

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
- **Money never computed in the UI.** TypeScript enforced this with a branded
  `Cents` type — arithmetic compiles but the result cannot be sent back. Dart
  has no such brand by default: use an extension type, or the rule evaporates
  silently.
- **A row with an unparseable amount is omitted, never sent as zero.**
- **Hide-all is answered in the UI**, because the core reads an empty filter set
  as "no filter".

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
