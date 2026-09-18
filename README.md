# Raven Budgeting

A local-first expense tracker and budgeting app. Rust core, Flutter app.

## What it does

Record receipts as they happen — including while traveling — have that spending
automatically become "actual" against a budget, and export everything to a spreadsheet
filterable by month and year. Budgets handle annually-paid items like insurance without
wrecking a single month's report.

Unlike bank-sync apps, this captures **line-item receipt data**. A bank only ever sees
`KROGER $142.87`; this sees what was actually in the basket.

## Sales tax

Receipts are entered as a subtotal per category; the tax is worked out from where
the store is. Those rates come from the Utah State Tax Commission's [quarterly
publication][utah-rates] — every taxing jurisdiction in the state, not a list
typed in by hand — and unprepared food is taxed at Utah's reduced 3% rather than
the general rate, so a $7.28 grocery subtotal owes $0.22 and not $0.53.

**Utah only, for now.** The rates are one state's publication in one state's file
format. Another state means another publisher, schedule and layout, so the honest
scope is the one state whose file is actually read. Anywhere else can still be
named with its rate typed in.

Rates change on the first day of each quarter. The app checks once the quarter
turns and downloads the new file if it can reach the network; being offline is
normal rather than an error, and the setup screen names the quarter on screen so
a table that stopped updating is visible. `pacc rates` shows the same thing, and
`pacc rates --update` fetches on demand.

None of this rewrites history: every transaction records the rate used when it
was entered, so a refresh changes what the *next* receipt is taxed at and never
what last March's was.

[utah-rates]: https://tax.utah.gov/business/sales-tax/sales/rates/

## Layout

```
crates/accounting-core/   The product: domain, storage, export. No UI.
crates/accounting-cli/    `pacc` — headless CLI. Debug tool and boundary enforcer.
app/                      The Flutter app. `app/rust/` is its bridge to the core.
```

`accounting-core` never touches a UI framework, an async runtime, a path-discovery
crate, or the system clock. See the crate docs for why — each rule maps to a mobile
portability failure.

## Development

Requires the toolchain pinned in `rust-toolchain.toml` (installed automatically by
rustup), plus Flutter for the app.

```sh
cargo test --workspace                 # core + CLI
cargo run -p accounting-cli -- --help
cargo install --path crates/accounting-cli   # installs `pacc`
```

### The Flutter app

The frontend, and the one that goes to Android and iOS. It reads the same
`pacc.db` layout the CLI does, so `pacc doctor`, `pacc export` and `pacc backup`
all work against it — point `--db` at the app data directory.
`docs/flutter-port.md` records how it got here and what is left.

Needs Flutter on `PATH` and `flutter_rust_bridge_codegen` from cargo. The
generated Dart under `app/lib/src/rust/` is committed and never edited by
hand; regenerate it with `flutter_rust_bridge_codegen generate` after changing
anything under `app/rust/src/api/`.

```sh
cd app
flutter pub get
flutter test                                    # widget tests, no device
flutter test integration_test/ -d linux         # calls the real core
flutter run -d linux
```

`app/rust` is a workspace member, so `cargo build --workspace` compiles it —
but that proves only that it compiles. It is a `cdylib`: nothing links it into
an application until Flutter's own build runs, so a change there is not tested
until `flutter test integration_test/` has run.

### Showing it off

`scripts/demo.sh` builds a throwaway database and opens the app on it — eight
months of receipts, twenty-one budgets, a fund waiting to be set aside and an
inbox with work in it. Every row goes in through `pacc`, so the figures on
screen are the real core's arithmetic and only the receipts are invented.

```sh
scripts/demo.sh                    # build, then run on the Linux desktop
scripts/demo.sh --device android   # build, install, and push to a connected phone
scripts/demo.sh --build-only       # just the database
```

It writes to `$TMPDIR/pacc-demo-$(id -u)/pacc.db` — named for your user
because `/tmp` is shared — never to the database the app normally opens, and
rebuilds it from nothing every run. Set `PACC_DEMO_DIR` to put it elsewhere.

The demo is its own app, **Raven Demo**, on every platform, so it can be open
beside Raven Budgeting and never sees the real app's data. One flag does it:
`--dart-define=PACC_DEMO=true`, which `demo.sh` passes. On Android the build
is `com.tannerhathaway.app.demo`, installed beside the real app rather than
over it. On Linux it is GTK application `com.tannerhathaway.app.demo` in a
window titled Raven Demo; on Windows the window and the file properties say
Raven Demo; on macOS the bundle is `com.tannerhathaway.app.demo`. An iOS demo
also needs the id as a build setting, which the build asks for rather than
guess — `FLUTTER_XCODE_PRODUCT_BUNDLE_IDENTIFIER=com.tannerhathaway.app.demo`
(see `app/tool/demo_bundle.sh`).

On the desktop the demo keeps its data in `personal-accounting-demo` beside
the real `personal-accounting` directory, and reads `$PACC_DEMO_DB` rather
than `$PACC_DB` — which names the real books, so a demo that honoured it could
open them. `demo.sh` sets `PACC_DEMO_DB` to the database it just built.

The amounts wander so the screens do not read as generated, but they wander
identically every time: a demo that shows different figures on each run
cannot be rehearsed.

### The web app

The same app as a static page: the core compiled to WebAssembly, the database
SQLite inside the browser's IndexedDB. There is no server behind it and
nothing is sent anywhere — no account, no analytics, no fonts or scripts from
anybody else's servers. The page talks to its own origin and nothing else,
and its Content-Security-Policy (in `app/web/index.html`) has the browser
enforce that rather than take the code's word for it.

```sh
scripts/build-web.sh                   # Rust → app/web/pkg/, Flutter → app/build/web, then checks
scripts/publish-web.sh ../raven-site   # copy into a clone of the site repo; prints the git commands
```

`build-web.sh` needs what `scripts/build-web-bridge.sh` lists (the
`wasm32-unknown-unknown` target, clang, `wasm-bindgen-cli`), and fails the
build if the output mentions a host it has not been told about, has lost its
policy, or contains anything shaped like data. To try a build locally:
`python3 -m http.server -d app/build/web 8080` and open
`http://localhost:8080` — localhost, because the storage and locking APIs the
app needs exist only in a secure context, and a LAN address is not one.

What is different in a browser:

- **Sync is the sealed file and nothing else.** Export downloads it and
  Import opens the browser's file picker. A page cannot tell a saved download
  from a cancelled one, so an export is not counted as sent until you press
  *Mark as sent*. There is no Wi-Fi sync: a page has no sockets.
- **A new browser starts by joining with a file.** On the desktop or phone
  that has your records, Settings → Sync → *For a new device* writes a copy
  of everything; in the browser, *Join with a file* takes that copy in place
  of the starting categories it seeded. The ordinary export file will not do
  for this — once a device is paired it holds only what changed since the
  last exchange — and joining from it is refused before anything is replaced.
  After joining, the two exchange files as usual.
- **Rates come from a file you download.** The Tax Commission's server does
  not allow pages from other sites to read it, and the policy would not let
  the page ask. In Settings, *Import rate workbook…* installs the quarter's
  spreadsheet once you have downloaded it from tax.utah.gov.
- **One tab at a time.** Each tab keeps its own copy of the database in memory
  and writes it back, so two at once would corrupt it. A second tab is turned
  away with a message saying so; close the other and reload.
- Characters outside Roboto — CJK, emoji — show as boxes. The engine would
  otherwise fetch fallback fonts from Google.

#### Your data can disappear

A browser's storage belongs to the browser. **Safari deletes everything a site
has stored after seven days of use without a visit to it**, on macOS and iOS
alike. Clearing browsing data deletes it in any browser, and a private window
forgets it on closing. The app asks for persistent storage, which Chrome, Edge
and Firefox may grant, but nothing overrides Safari's rule. Settings says so
beside the sync section; treat the browser as one device of a pair, never the
only copy.

#### Hosting: a GitHub organisation of its own

GitHub Pages is free and serves static files, which is all this needs. The
catch is the origin. A browser keeps storage per origin, and every Pages site
under one account shares that account's: `<user>.github.io/raven` and
`<user>.github.io/anything-else` are the same origin, so any page ever
published under the account could open Raven's database and read the ledger
in it. So the site gets an origin nobody else can publish to:

1. Create a free GitHub organisation used for this and nothing else — no other
   repositories with Pages, now or later.
2. In it, create a **public** repository named `<org>.github.io` (free Pages
   needs a public repository). It holds the built site only: never source,
   never data. `publish-web.sh` refuses to copy anything that looks like a
   database, a sync file or an export, into it or out of the build.
3. In its settings, under Pages, deploy from the `main` branch, root folder.
4. Clone it beside this repository. Each release is then `build-web.sh`,
   `publish-web.sh ../<org>.github.io`, a look at `git status`, a commit and a
   push.

The app is served from the root of `https://<org>.github.io/`, which is why
the build's base href is `/`. The source repository stays wherever it is and
is never published.

Pages cannot set response headers, so the policy is a `<meta>` tag, and a
`<meta>` policy cannot forbid framing. Another site can embed the page, but in
current browsers an embedded copy gets storage of its own — an empty
database — rather than yours.

## Platform status

| Platform | Status |
|---|---|
| Linux | Primary target |
| Windows | Planned — build on Windows or CI, not cross-compiled |
| Android | Planned — buildable from Linux via `cargo-ndk` |
| iOS | Designed for, **not buildable without macOS** (Xcode is macOS-only) |
| Web | Static site on GitHub Pages — see [The web app](#the-web-app) |
