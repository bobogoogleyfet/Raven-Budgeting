# Personal Accounting

A local-first expense tracker and budgeting app. Rust core, Tauri desktop UI.

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
crates/accounting-tauri/  Thin IPC shim. Excluded from the workspace — see below.
ui/                       Vite + TypeScript frontend.
```

`accounting-core` never touches a UI framework, an async runtime, a path-discovery
crate, or the system clock. See the crate docs for why — each rule maps to a mobile
portability failure.

## Development

Requires the toolchain pinned in `rust-toolchain.toml` (installed automatically by
rustup) and Node per `.nvmrc` for the frontend.

```sh
cargo test --workspace                 # core + CLI
cargo run -p accounting-cli -- --help
cargo install --path crates/accounting-cli   # installs `pacc`

npm --prefix ui install
npm --prefix ui test                   # the money formatting/parsing boundary
npm --prefix ui run build
```

### The desktop app

`crates/accounting-tauri` is a workspace member, so it needs the webkit development
headers to be installed — `tauri`'s build script links against them, and a member that
cannot build stops `cargo test --workspace` from resolving for *every* crate, taking
the core's tests down with it. On a machine without them, move the crate from `members`
into `exclude` in the root `Cargo.toml` and the rest of the workspace still works.

```sh
sudo apt install -y libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev \
  libsoup-3.0-dev build-essential curl wget file libxdo-dev libssl-dev \
  libayatana-appindicator3-dev librsvg2-dev pkg-config
```

Then:

```sh
npm --prefix ui install
npm --prefix ui run build        # required: the frontend is embedded at compile time
cargo run -p accounting-tauri
```

There is deliberately no `devUrl` in `tauri.conf.json`. With one set, a debug build
points the webview at the Vite dev server rather than the bundled frontend, so
`cargo run` opens a window reading *"Could not connect to localhost: Connection
refused"* unless `npm run dev` happens to be running elsewhere. The cost of leaving it
out is that a change under `ui/` needs `npm --prefix ui run build` before the next
`cargo run`; `build.rs` watches `ui/dist` so cargo notices.

The desktop app reads the same `pacc.db` layout the CLI uses, so `pacc doctor`,
`pacc export` and `pacc backup` all work against it — point `--db` at the app data
directory.

### The Flutter app

The frontend that will replace the Tauri shell, and the one that goes to
Android and iOS. Phase 1 of `docs/flutter-port.md`: it opens the same
`pacc.db` the CLI and the Tauri app use, and lists the seeded categories.

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

## Platform status

| Platform | Status |
|---|---|
| Linux | Primary target |
| Windows | Planned — build on Windows or CI, not cross-compiled |
| Android | Planned — buildable from Linux via `cargo-ndk` |
| iOS | Designed for, **not buildable without macOS** (Xcode is macOS-only) |
