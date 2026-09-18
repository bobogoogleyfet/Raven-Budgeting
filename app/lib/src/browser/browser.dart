/// What the app needs from wherever it runs: a desktop or phone, or a page in
/// a browser.
///
/// # Why this is one seam and not `kIsWeb` checks scattered about
///
/// The browser half touches `dart:js_interop`, which a native build cannot
/// compile, and the native half touches `dart:io` and `path_provider`, which
/// compile for the browser and then throw the moment they are called. A
/// conditional export keeps each half out of the other's build, so a native
/// build never carries a line of browser code and nothing in the page can
/// reach for a filesystem it does not have.
///
/// Callers import this file and nothing else, so a screen reads the same on
/// every platform and asks `kIsWeb` only about what it *shows*.
library;

export 'browser_native.dart' if (dart.library.js_interop) 'browser_web.dart';
export 'picked_file.dart';
