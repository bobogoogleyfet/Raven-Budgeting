use std::path::Path;

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
    let dist = Path::new("../../ui/dist");
    println!("cargo:rerun-if-changed=../../ui/dist");

    if !dist.join("index.html").exists() {
        // Failing here beats shipping a window that renders nothing, which
        // looks like a broken app rather than a missing build step.
        panic!(
            "the frontend bundle is missing: {}\n\
             build it first:  npm --prefix ui install && npm --prefix ui run build",
            dist.join("index.html").display()
        );
    }

    tauri_build::build();
}
