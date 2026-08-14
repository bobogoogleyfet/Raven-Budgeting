use std::path::Path;
use std::time::SystemTime;

fn main() {
    // There is deliberately no `devUrl` in tauri.conf.json. With one set, a
    // debug build points the webview at the Vite dev server instead of the
    // bundled frontend — so `cargo run` opens a window showing "Could not
    // connect to localhost: Connection refused" unless `npm run dev` happens
    // to be running in another terminal. Loading `frontendDist` in both
    // profiles means `cargo run` simply works.
    //
    // The cost is that the frontend is embedded at compile time, so a change
    // under ui/ needs `npm --prefix ui run build` and then a rebuild. The
    // rerun-if-changed below makes cargo notice the rebuilt bundle; without
    // it, cargo sees no Rust change and keeps the stale assets.
    let ui = Path::new("../../ui");
    let dist = ui.join("dist");
    println!("cargo:rerun-if-changed=../../ui/dist");
    // Watched as well as dist, so that editing a .ts file re-runs this script
    // and reaches the staleness check below. Without it cargo has no reason to
    // look at the frontend at all.
    println!("cargo:rerun-if-changed=../../ui/src");

    if !dist.join("index.html").exists() {
        // Failing here beats shipping a window that renders nothing, which
        // looks like a broken app rather than a missing build step.
        panic!(
            "the frontend bundle is missing: {}\n\
             build it first:  npm --prefix ui install && npm --prefix ui run build",
            dist.join("index.html").display()
        );
    }

    // # Why a stale bundle has to be an error
    //
    // `cargo run` rebuilds Rust and never rebuilds the frontend, so a merge
    // that brings new TypeScript leaves `ui/dist` holding the *old* screen.
    // The app then starts and works — with new arithmetic behind an old
    // interface. That is the worst shape a failure can take: the numbers
    // change, the layout does not, and it reads as "the fix didn't work"
    // rather than "a build step was skipped". It cost several rounds of
    // exactly that confusion before this check existed.
    if let (Some(source), Some(built)) = (newest_mtime(&ui.join("src")), newest_mtime(&dist))
        && source > built
    {
        panic!(
            "the frontend bundle is older than ui/src — the window would show the previous UI\n\
             rebuild it:  npm --prefix ui run build"
        );
    }

    tauri_build::build();
}

/// The most recent modification time anywhere under `dir`.
///
/// `None` when the directory cannot be read, which is not treated as staleness:
/// a check that cannot see the files has no opinion, and guessing would fail
/// builds for a reason the message could not explain.
fn newest_mtime(dir: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(&path).ok()?.flatten() {
            let kind = entry.file_type().ok()?;
            if kind.is_dir() {
                stack.push(entry.path());
            } else if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                newest = Some(newest.map_or(modified, |best: SystemTime| best.max(modified)));
            }
        }
    }
    newest
}
