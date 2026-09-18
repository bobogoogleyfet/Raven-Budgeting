//! What the web build does instead of opening a file.
//!
//! A web page has no filesystem, so the database lives in the browser's
//! IndexedDB, under this site's origin, and never leaves the machine: nothing
//! in this module, or anywhere in the browser build, talks to a network. The
//! core does not know the difference — it is handed a name, exactly as it is
//! handed a path everywhere else.

use std::cell::Cell;

use js_sys::{Function, Object, Promise, Reflect};
use sqlite_wasm_vfs::relaxed_idb::{self, Preload, RelaxedIdbCfg};
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;

/// The IndexedDB database the pages are kept in, and the prefix of the lock
/// that guards them.
///
/// Named for the app rather than left at the VFS's default: a GitHub Pages
/// origin is shared by every project page its owner publishes, so a generic
/// name is one another page could plausibly also pick.
const STORE: &str = "raven-budgeting";

thread_local! {
    /// Whether this page holds the lock [`claim`] takes. A page has one thread,
    /// so thread-local is page-wide.
    static CLAIMED: Cell<bool> = const { Cell::new(false) };
}

/// The name the database file goes by inside [`STORE`].
///
/// The demo gets a file of its own for the same reason it gets a directory of
/// its own on a desktop: a demonstration must never be able to write into the
/// real books, and on the web the two can be served from one origin.
pub(crate) fn database_file(demo: bool) -> &'static str {
    if demo { "pacc-demo.db" } else { "pacc.db" }
}

/// Make `file` openable by SQLite: take this tab's claim on it, then load it
/// out of IndexedDB and make that the default VFS.
///
/// Must finish before `Db::open`, which is synchronous and cannot wait for
/// IndexedDB itself; that is the whole reason this step is separate.
pub(crate) async fn prepare(file: &str) -> Result<(), String> {
    claim(file).await?;

    let config = RelaxedIdbCfg {
        vfs_name: STORE.to_string(),
        // Only the file this build opens. The other one, demo or real, stays
        // in IndexedDB and out of memory.
        preload: Preload::Paths(vec![file.to_string()]),
        ..RelaxedIdbCfg::default()
    };
    relaxed_idb::install::<sqlite_wasm_rs::WasmOsCallback>(&config, true)
        .await
        .map_err(|e| format!("the browser's storage could not be opened: {e}"))?;
    Ok(())
}

/// Refuse to open a database another tab already has open.
///
/// # Why this is not optional
///
/// Each tab loads the whole file into its own memory and writes the pages it
/// changes back to IndexedDB. Two tabs are two copies drifting apart, and
/// their write-backs interleave page by page, so the file on disk ends up
/// half one copy and half the other — not a lost receipt but a corrupt
/// database. The Web Locks API is held for as long as the page is alive and
/// released by the browser when it goes, however it goes, so a crashed or
/// closed tab never leaves the database locked.
///
/// A browser without Web Locks (they need a secure context, which GitHub
/// Pages and `localhost` both are) opens without the guard rather than not at
/// all; there is nothing it could do instead.
async fn claim(file: &str) -> Result<(), String> {
    // This page's own claim, from an earlier open that took the lock and then
    // failed further on — a migration, say — and is being tried again. Asking
    // a second time would find the lock taken, by us, and turn the page away
    // from its own database.
    if CLAIMED.with(Cell::get) {
        return Ok(());
    }

    let locks = Reflect::get(&js_sys::global(), &"navigator".into())
        .and_then(|navigator| Reflect::get(&navigator, &"locks".into()))
        .unwrap_or(JsValue::UNDEFINED);
    if locks.is_undefined() || locks.is_null() {
        log::warn!("this browser has no Web Locks; a second tab is not kept out");
        return Ok(());
    }

    // `request` settles only when the lock is let go, which for a granted lock
    // is never, so it cannot be what this waits on. The callback reports
    // whether it was granted through a promise of its own, and then hands the
    // lock back a promise that never settles, which holds it for the life of
    // the page.
    let mut settle: Option<Function> = None;
    let granted = Promise::new(&mut |resolve, _reject| settle = Some(resolve));
    let settle = settle.ok_or("the browser did not run a promise's executor")?;
    let callback = Closure::once_into_js(move |lock: JsValue| -> JsValue {
        let held = !lock.is_null();
        let _ = settle.call1(&JsValue::NULL, &JsValue::from_bool(held));
        if held {
            Promise::new(&mut |_, _| {}).into()
        } else {
            JsValue::UNDEFINED
        }
    });

    // `ifAvailable`: answer at once rather than queue behind the other tab,
    // which would leave this one on a blank screen for as long as that one
    // stays open.
    let options = Object::new();
    Reflect::set(&options, &"ifAvailable".into(), &JsValue::TRUE).map_err(describe)?;
    let request: Function = Reflect::get(&locks, &"request".into())
        .map_err(describe)?
        .into();
    request
        .call3(
            &locks,
            &format!("{STORE}/{file}").into(),
            &options,
            &callback,
        )
        .map_err(describe)?;

    let held = JsFuture::from(granted).await.map_err(describe)?;
    if held.as_bool() == Some(true) {
        CLAIMED.with(|claimed| claimed.set(true));
        Ok(())
    } else {
        Err("Raven Budgeting is already open in another tab or window. \
             Close it, then reload this page."
            .to_string())
    }
}

/// Milliseconds since the epoch, from JavaScript's clock.
///
/// `SystemTime::now` panics on this target: the standard library has no
/// clock to read here, and `Date.now()` is the one there is. It is a whole
/// number of milliseconds held in a double, so the cast only drops a
/// fractional part that is always zero, and a clock before 1970 reads as 0
/// for the same reason as on every other platform.
pub(crate) fn now_millis() -> u64 {
    js_sys::Date::now() as u64
}

/// A JavaScript exception, as text a person can read.
fn describe(error: JsValue) -> String {
    error
        .as_string()
        .or_else(|| {
            Reflect::get(&error, &"message".into())
                .ok()
                .and_then(|m| m.as_string())
        })
        .unwrap_or_else(|| format!("{error:?}"))
}
