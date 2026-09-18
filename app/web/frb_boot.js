// Starts the app's Rust wasm without flutter_rust_bridge's own loader.
//
// That loader injects pkg/accounting_bridge.js and then runs
// `window.wasm_bindgen = wasm_bindgen` through `new Function(...)`, which
// needs 'unsafe-eval' in the page's Content-Security-Policy. index.html loads
// the same script directly instead, and this does the rest: publish the
// binding where the bridge looks for it, and start instantiating the module
// while Flutter is still loading. The Dart side awaits `__frbReady` before
// telling the bridge the library is already there.
//
// Relative to the page's <base>, like everything else on it.
window.wasm_bindgen = wasm_bindgen;
window.__frbReady = wasm_bindgen({ module_or_path: 'pkg/accounting_bridge_bg.wasm' });
