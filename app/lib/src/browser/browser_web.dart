/// The browser half of `browser.dart`: a page, with no filesystem, that any
/// number of tabs can have open at once.
library;

import 'dart:async';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data' show Uint8List;

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show ExternalLibrary;
import 'package:web/web.dart' as web;

import '../rust/frb_generated.dart';
import 'picked_file.dart';

/// Set by `web/frb_boot.js`: the promise of the Rust wasm being instantiated.
@JS('__frbReady')
external JSPromise<JSAny?>? get _bridgeReady;

/// Attach to the Rust library the page has already loaded.
///
/// flutter_rust_bridge's own web loader injects the wasm-bindgen script and
/// then runs `window.wasm_bindgen = wasm_bindgen` through `new Function`,
/// which needs `'unsafe-eval'` in the Content-Security-Policy — and a page
/// holding a household's ledger is exactly the page that policy should be
/// strictest on. So `index.html` loads the same two files as ordinary
/// same-origin scripts, `frb_boot.js` starts the instantiation, and all that
/// is left here is to wait for it and hand the bridge a library it need not
/// fetch.
Future<void> startRust() async {
  final ready = _bridgeReady;
  if (ready == null) {
    throw StateError(
      'The Rust half of the app did not load. pkg/accounting_bridge.js is '
      'missing from this build, or the browser refused it.',
    );
  }
  await ready.toDart;
  await RustLib.init(
    // The analyzer resolves conditional imports as a native build would, and
    // sees the native `ExternalLibrary`, whose constructor is private. This
    // file is only ever compiled for the web, where the class is the one
    // above with a public `debugInfo` constructor.
    // ignore: const_with_undefined_constructor_default
    externalLibrary: const ExternalLibrary(debugInfo: 'loaded by frb_boot.js'),
  );
}

/// The name every tab of this origin queues on. Anything else on the origin
/// using the same name would share the queue, which is one more reason the
/// site gets an origin of its own.
const String _lockName = 'raven-database';

/// Never completed: a Web Lock is held until the promise its callback returned
/// settles, and this tab means to hold it until it closes.
final Completer<void> _forever = Completer<void>();

/// Be the only tab with the database open, or wait until this one can be.
///
/// # Why a lock and not a warning
///
/// The database is SQLite on top of IndexedDB, and the VFS underneath keeps
/// its working copy in each tab's memory and writes it back behind the scenes.
/// Two tabs each writing back their own copy do not produce a conflict
/// somebody can resolve — they produce a file that is quietly wrong. So a
/// second tab does not open the database at all.
///
/// The browser releases a Web Lock when the tab holding it closes, crashes or
/// navigates away, so a lock cannot be leaked the way a flag written to
/// storage can. And since a waiting request is granted the moment the other
/// tab goes, [whileWaiting] shows a page that says why, and this returns —
/// and the app opens — as soon as it is this tab's turn.
///
/// False when the browser cannot promise exclusivity at all: no Web Locks, or
/// a page context (a sandboxed frame, an opaque origin) that is refused them.
/// Every browser that can run the app has them, so that case is refused
/// rather than risked.
Future<bool> claimDatabase({required void Function() whileWaiting}) async {
  final web.Navigator navigator = web.window.navigator;
  if (!(navigator as JSObject).has('locks')) return false;
  final locks = navigator.locks;

  if (await _request(locks, ifAvailable: true)) return true;
  whileWaiting();
  return _request(locks, ifAvailable: false);
}

/// Ask for the lock once. True when it is held, false when `ifAvailable`
/// found it taken or the request was refused outright.
Future<bool> _request(web.LockManager locks, {required bool ifAvailable}) {
  final granted = Completer<bool>();

  JSPromise<JSAny?> onGrant(web.Lock? lock) {
    if (lock == null) {
      granted.complete(false);
      return Future<void>.value().toJS;
    }
    granted.complete(true);
    return _forever.future.toJS;
  }

  locks
      .request(
        _lockName,
        web.LockOptions(mode: 'exclusive', ifAvailable: ifAvailable),
        onGrant.toJS,
      )
      .toDart
      .then<void>(
    (_) {},
    onError: (Object _) {
      if (!granted.isCompleted) granted.complete(false);
    },
  );
  return granted.future;
}

/// Ask the browser not to clear this site's storage when it is short of space.
///
/// Without this, everything the user has entered is "best-effort" storage,
/// which a browser may drop under pressure without asking anybody. Chrome and
/// Edge decide silently from how the site is used; Firefox asks the user.
/// Either answer is fine to carry on with — which is why nothing here reports
/// it — but it is worth asking, once, before anything has been written.
///
/// It does not help with Safari, which clears a site's storage after seven
/// days without a visit whatever it was told. The settings screen says so.
Future<void> askToKeepStorage() async {
  try {
    final storage = web.window.navigator.storage;
    if ((await storage.persisted().toDart).toDart) return;
    await storage.persist().toDart;
  } catch (_) {
    // An insecure context has no StorageManager, and a refusal is an answer.
    // Neither stops the app from working today.
  }
}

/// The browser has no directory to offer: the database lives in the origin's
/// IndexedDB, which Rust opens by name.
Future<String?> appDataDirectory() async => null;

/// Only ever used to build a suggested path, which the page never shows.
String get pathSeparator => '/';

/// There is no documents directory a page may name. The sync screen offers a
/// download instead of a path in a browser, so this is never asked.
Future<String> documentsDirectory() async =>
    throw UnsupportedError('A page has no documents directory.');

/// Never reached in a browser, for the same reason as [documentsDirectory].
Future<void> writeNewFile(String path, Uint8List bytes) async =>
    throw UnsupportedError('A page cannot write to a path.');

/// Never reached in a browser, for the same reason as [documentsDirectory].
Future<Uint8List> readFileBytes(String path) async =>
    throw UnsupportedError('A page cannot read from a path.');

/// Hand [bytes] to the browser as a download called [name].
///
/// The ordinary anchor-with-`download` dance: the bytes never leave the page
/// except into the browser's own download manager, and nothing is fetched —
/// the object URL is local to this document. It is revoked a minute later
/// rather than at once, because some browsers read it after the click returns
/// and a revoked URL there is a download that silently fails.
Future<void> downloadFile(String name, Uint8List bytes) async {
  final blob = web.Blob(
    <web.BlobPart>[bytes.toJS].toJS,
    web.BlobPropertyBag(type: 'application/octet-stream'),
  );
  final url = web.URL.createObjectURL(blob);
  final anchor = web.HTMLAnchorElement()
    ..href = url
    ..download = name;
  anchor.style.display = 'none';
  web.document.body!.append(anchor);
  anchor.click();
  anchor.remove();
  Timer(const Duration(minutes: 1), () => web.URL.revokeObjectURL(url));
}

/// Ask the user for one file, through the browser's own picker. Null when
/// they close it without choosing.
///
/// Must be called straight from a button press, before anything is awaited:
/// a browser opens a file picker only in response to a gesture it can see,
/// and an `await` in between is enough for Safari to decide there was none.
/// The body below runs synchronously up to `input.click()` for that reason.
///
/// A browser that never fires `cancel` leaves a dismissed picker's future
/// pending forever, which costs one listener and nothing else — the caller
/// does not mark itself busy until a file has actually arrived.
Future<PickedFile?> pickFile() async {
  final input = web.HTMLInputElement()..type = 'file';
  input.style.display = 'none';
  web.document.body!.append(input);

  final chosen = Completer<web.File?>();
  final changes = web.EventStreamProviders.changeEvent.forTarget(input).listen((
    _,
  ) {
    final files = input.files;
    if (chosen.isCompleted) return;
    chosen.complete(files != null && files.length > 0 ? files.item(0) : null);
  });
  final cancels = const web.EventStreamProvider<web.Event>(
    'cancel',
  ).forTarget(input).listen((_) {
    if (!chosen.isCompleted) chosen.complete(null);
  });

  input.click();
  try {
    final file = await chosen.future;
    if (file == null) return null;
    final buffer = await file.arrayBuffer().toDart;
    return PickedFile(name: file.name, bytes: buffer.toDart.asUint8List());
  } finally {
    await changes.cancel();
    await cancels.cancel();
    input.remove();
  }
}
