// The browser half of the seam, in a real browser:
//
//   flutter test --platform chrome test/browser_web_test.dart
//
// Skipped by an ordinary `flutter test`, which runs on the VM where none of
// this exists.
@TestOn('browser')
library;

import 'dart:typed_data';

import 'package:app/src/browser/browser.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('a second claim waits while the first is held', () async {
    // One page stands in for two tabs: a Web Lock is per origin, not per tab,
    // so a second request from the same page queues exactly as another tab's
    // would.
    expect(await claimDatabase(whileWaiting: () => fail('first should not wait')),
        isTrue);

    var waited = false;
    final second = claimDatabase(whileWaiting: () => waited = true);
    // Still queued behind the first claim, which this page never releases.
    final outcome = await Future.any<Object?>([
      second,
      Future<Object?>.delayed(const Duration(milliseconds: 300), () => 'queued'),
    ]);

    expect(waited, isTrue);
    expect(outcome, 'queued');
  });

  test('asking to keep storage never throws, whatever the answer', () async {
    await askToKeepStorage();
  });

  test('a download is handed over without an error', () async {
    // What becomes of it is the browser's business and invisible to the page,
    // which is the whole reason the sync screen asks the user.
    await downloadFile('test.paccsync', Uint8List.fromList(const [1, 2, 3]));
  });

  test('there is no directory to offer', () async {
    expect(await appDataDirectory(), isNull);
    expect(documentsDirectory(), throwsUnsupportedError);
  });
}
