import 'dart:typed_data' show Uint8List;

/// Asks for one file, or null when the user closes the picker. The browser's
/// is `pickFile` in `browser.dart`; tests pass their own.
typedef FilePicker = Future<PickedFile?> Function();

/// A file somebody chose in the browser's own picker.
///
/// The name comes along because it is the only thing the page is told about
/// where the bytes came from — a browser never hands over a path — and a
/// message that says which file was read is how a person notices they picked
/// last week's.
class PickedFile {
  const PickedFile({required this.name, required this.bytes});

  final String name;
  final Uint8List bytes;
}
