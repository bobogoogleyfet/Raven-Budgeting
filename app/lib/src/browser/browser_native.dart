/// The native half of `browser.dart`: a desktop or a phone, with a filesystem
/// and one process per app.
library;

import 'dart:io';
import 'dart:typed_data' show Uint8List;

import 'package:path_provider/path_provider.dart';

import '../rust/frb_generated.dart';
import 'picked_file.dart';

/// Load the Rust library the ordinary way: the platform finds it beside the
/// executable, or inside the app bundle.
Future<void> startRust() => RustLib.init();

/// Nothing evicts an app's own files on a desktop or a phone.
Future<void> askToKeepStorage() async {}

/// Where the database lives, when only the platform can say.
///
/// Desktop resolves the path in Rust, from the same environment variables the
/// CLI reads, so the app and `pacc` share one database — hence null.
///
/// Mobile cannot: there is no shared location to share *with*, and the
/// per-app directory is the platform's answer — obtainable only through the
/// Java `Context`, which is what `path_provider` wraps. Support rather than
/// documents: this is a database the app manages, not a file the user is
/// meant to browse to, and on iOS the documents directory is backed up and
/// user-visible in ways a SQLite file with a WAL beside it should not be.
Future<String?> appDataDirectory() async => Platform.isAndroid || Platform.isIOS
    ? (await getApplicationSupportDirectory()).path
    : null;

/// What goes between a directory and a file name here.
String get pathSeparator => Platform.pathSeparator;

/// Where to suggest writing a bundle.
///
/// `path_provider`, not a guess: only the platform knows where a user's
/// documents are, and on Android the answer is not a path any rule could
/// derive.
Future<String> documentsDirectory() async =>
    (await getApplicationDocumentsDirectory()).path;

/// Write a bundle that did not exist a moment ago.
///
/// Refuses to overwrite, exactly as `pacc sync export` does: a file you cannot
/// get back is data until its owner says otherwise, and the one thing a stale
/// sync file looks like is a fresh one. `dart:io` has no exclusive-create
/// mode, so the check is a courtesy rather than a lock — two exports racing
/// for one name is not a case worth more machinery than that.
///
/// `flush: true` because the caller marks the bundle delivered on the strength
/// of this returning, and "the bytes are in the page cache" is not the same
/// promise as "the bytes are on the stick you are about to pull out".
Future<void> writeNewFile(String path, Uint8List bytes) async {
  final file = File(path);
  if (await file.exists()) {
    throw '$path already exists, and a sync file is never overwritten. '
        'Give this one another name, or delete that one first.';
  }
  await file.parent.create(recursive: true);
  await file.writeAsBytes(bytes, flush: true);
}

/// Read a bundle off disk.
Future<Uint8List> readFileBytes(String path) => File(path).readAsBytes();

/// Never reached: a native build writes to a path the user typed. Here so the
/// two halves of the seam agree on their shape, and loud if that ever changes.
Future<void> downloadFile(String name, Uint8List bytes) =>
    throw UnsupportedError('Downloads exist only in a browser.');

/// Never reached, for the same reason as [downloadFile].
Future<PickedFile?> pickFile() =>
    throw UnsupportedError('The browser file picker exists only in a browser.');

/// Never reached: only a page is turned away for being open twice.
void reloadPage() =>
    throw UnsupportedError('Only a page can be reloaded.');
