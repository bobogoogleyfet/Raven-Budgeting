import 'dart:typed_data';

import 'package:app/src/api.dart';
import 'package:app/src/browser/browser.dart' show PickedFile;
import 'package:app/src/screens/settings_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// What a test's fake bridge was asked to do, in the order it was asked.
///
/// Order is half of what these tests are checking: a bundle must not be
/// recorded as sent before the bytes are on disk, because a file that never
/// reached the other device would otherwise stop its contents from ever being
/// sent again.
class Calls {
  final List<String> log = [];
  final List<String> written = [];
  final List<String?> markedFor = [];
  final List<String> downloaded = [];
  final List<Uint8List> imported = [];
}

Applied applied({
  int taken = 0,
  int ignored = 0,
  int conflicts = 0,
  List<String> deferred = const [],
  List<String> unknownKinds = const [],
}) =>
    Applied(
      applied: taken,
      ignored: ignored,
      conflicts: conflicts,
      deferred: deferred,
      unknownKinds: unknownKinds,
    );

Future<Calls> pump(
  WidgetTester tester, {
  List<String> peers = const ['phone-7f2c'],
  int openConflicts = 0,
  Object? statusError,
  Object? exportError,
  Object? writeError,
  Object? markError,
  Object? readError,
  Object? importError,
  Applied? importResult,
  Uint8List? bundle,
  bool browser = false,
  PickedFile? picked,
  Object? downloadError,
}) async {
  final calls = Calls();
  // Tall enough that the whole section is laid out: a Column that overflows
  // fails the test with a layout error rather than with the thing it is about.
  tester.view.physicalSize = const Size(1000, 3000);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);

  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: SingleChildScrollView(
          child: SyncSection(
            loadStatus: () async {
              if (statusError != null) throw statusError;
              return SyncStatus(
                thisDevice: 'laptop-91ab',
                peers: peers,
                openConflicts: openConflicts,
              );
            },
            export: ({String? peer, required String phrase}) async {
              calls.log.add('export');
              if (exportError != null) throw exportError;
              return bundle ?? Uint8List.fromList(List.filled(2048, 7));
            },
            markSent: ({
              String? peer,
              required String phrase,
              required Uint8List bytes,
            }) async {
              calls.log.add('mark');
              calls.markedFor.add(peer);
              if (markError != null) throw markError;
            },
            import: ({required Uint8List bytes, required String phrase}) async {
              calls.log.add('import');
              calls.imported.add(bytes);
              if (importError != null) throw importError;
              return importResult ?? applied();
            },
            writeFile: (path, bytes) async {
              calls.log.add('write');
              if (writeError != null) throw writeError;
              calls.written.add(path);
            },
            readFile: (path) async {
              calls.log.add('read $path');
              if (readError != null) throw readError;
              return Uint8List.fromList(const [1, 2, 3]);
            },
            // A test has no platform directory and must not go looking for
            // one; the suggestion is only ever a starting point anyway.
            defaultDirectory: () async => '/tmp/pacc',
            browser: browser,
            download: (name, bytes) async {
              calls.log.add('download');
              if (downloadError != null) throw downloadError;
              calls.downloaded.add(name);
            },
            pick: () async {
              calls.log.add('pick');
              return picked;
            },
          ),
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
  return calls;
}

Future<void> exportTo(
  WidgetTester tester, {
  String phrase = 'seven rusty kettles',
  String path = '/tmp/pacc/out.paccsync',
}) async {
  await tester.enterText(find.byKey(const Key('export-phrase')), phrase);
  await tester.enterText(find.byKey(const Key('export-path')), path);
  await tester.tap(find.widgetWithText(FilledButton, 'Export'));
  await tester.pumpAndSettle();
}

Future<void> importFrom(
  WidgetTester tester, {
  String phrase = 'seven rusty kettles',
  String path = '/tmp/pacc/in.paccsync',
}) async {
  await tester.enterText(find.byKey(const Key('import-phrase')), phrase);
  await tester.enterText(find.byKey(const Key('import-path')), path);
  await tester.tap(find.widgetWithText(FilledButton, 'Import'));
  await tester.pumpAndSettle();
}

/// Every rendered label on screen, which is where a secret would leak to.
///
/// Deliberately not the whole widget tree: the phrase is legitimately inside
/// the box it was typed into, and an assertion that failed on that would be
/// asserting the field does not work.
List<String> labels(WidgetTester tester) => [
      for (final text in tester.widgetList<Text>(find.byType(Text)))
        text.data ?? '',
      for (final text
          in tester.widgetList<SelectableText>(find.byType(SelectableText)))
        text.data ?? '',
    ];

void main() {
  testWidgets('names this device and the ones it syncs with', (tester) async {
    await pump(tester, peers: ['phone-7f2c']);

    // The name is the *database's*, and it is what a peer keeps a watermark
    // against, so it is shown rather than hidden behind "this device".
    expect(find.text('laptop-91ab'), findsOneWidget);
    expect(find.text('phone-7f2c'), findsOneWidget);
  });

  testWidgets('says so plainly when nothing has been paired', (tester) async {
    await pump(tester, peers: const []);

    expect(find.textContaining('No other device yet'), findsOneWidget);
  });

  testWidgets('export reports where it wrote, and how much', (tester) async {
    final calls = await pump(tester);

    await exportTo(tester, path: '/tmp/pacc/laptop.paccsync');

    expect(calls.written, ['/tmp/pacc/laptop.paccsync']);
    expect(
      find.textContaining('Wrote /tmp/pacc/laptop.paccsync (2.0 kB)'),
      findsOneWidget,
    );
    // Naming the peer matters: the file only carries what *that* device has
    // not seen, so a file for the wrong one is a file that syncs nothing.
    expect(find.textContaining('for phone-7f2c'), findsOneWidget);
  });

  testWidgets('nothing is marked as sent until the bytes are on disk',
      (tester) async {
    final calls = await pump(tester);

    await exportTo(tester);

    // Export builds, the file is written, and only then is the watermark
    // advanced. The other order would mean a file that never arrived stopping
    // its contents from ever being sent again.
    expect(calls.log, ['export', 'write', 'mark']);
    expect(calls.markedFor, [null]);
  });

  testWidgets('a write that fails records nothing as sent', (tester) async {
    final calls = await pump(tester, writeError: 'the disk is full');

    await exportTo(tester);

    expect(calls.log, ['export', 'write']);
    expect(find.textContaining('Could not write the file'), findsOneWidget);
    expect(find.textContaining('the disk is full'), findsOneWidget);
  });

  testWidgets('the first file of a pairing is not recorded as sent',
      (tester) async {
    // There is nobody to record it against: a watermark names a device, and
    // inventing one would make it the "only peer" every later export talked
    // about while the real device's mark stayed empty.
    final calls = await pump(tester, peers: const []);

    await exportTo(tester);

    expect(calls.log, ['export', 'write']);
    expect(find.textContaining('first file of a pairing'), findsOneWidget);
  });

  testWidgets('a file that was written but not recorded says which happened',
      (tester) async {
    await pump(tester, markError: 'the database is locked');

    await exportTo(tester, path: '/tmp/pacc/laptop.paccsync');

    // The bytes are on disk either way, which is the part that matters; the
    // cost of the rest is one re-send, and saying nothing would look like a
    // clean export whose next file mysteriously repeats itself.
    expect(
      find.textContaining('Wrote /tmp/pacc/laptop.paccsync'),
      findsOneWidget,
    );
    expect(
      find.textContaining('Could not record it as sent'),
      findsOneWidget,
    );
  });

  testWidgets('refuses to export without a phrase, and calls nothing',
      (tester) async {
    final calls = await pump(tester);

    await tester.enterText(
      find.byKey(const Key('export-path')),
      '/tmp/pacc/out.paccsync',
    );
    await tester.tap(find.widgetWithText(FilledButton, 'Export'));
    await tester.pumpAndSettle();

    // The phrase is the only thing protecting a whole ledger left in a synced
    // folder, so an empty one is refused here rather than sealing a file
    // anybody can open.
    expect(calls.log, isEmpty);
    expect(find.textContaining('Enter the pairing phrase'), findsOneWidget);
  });

  testWidgets('with several peers it asks which, rather than guessing',
      (tester) async {
    final calls = await pump(tester, peers: ['phone-7f2c', 'desk-3319']);

    await exportTo(tester);

    expect(calls.log, isEmpty);
    expect(find.textContaining('Choose'), findsOneWidget);
  });

  testWidgets('import reports what was taken, ignored and disagreed over',
      (tester) async {
    await pump(
      tester,
      importResult: applied(taken: 12, ignored: 3, conflicts: 2),
    );

    await importFrom(tester, path: '/tmp/pacc/phone.paccsync');

    expect(find.text('Applied 12'), findsOneWidget);
    expect(find.text('Ignored 3'), findsOneWidget);
    expect(find.text('Conflicts 2'), findsOneWidget);
    // Losing a conflict is not being wrong, and the screen has to say so or a
    // user reads "conflicts 2" as "two things broke".
    expect(
      find.textContaining('does not mean that version was wrong'),
      findsOneWidget,
    );
    expect(find.textContaining('Read /tmp/pacc/phone.paccsync'), findsOneWidget);
  });

  testWidgets('deferred changes are listed in full, never as a count',
      (tester) async {
    await pump(
      tester,
      importResult: applied(
        taken: 4,
        deferred: [
          'txn 0199-aaaa needs merchant 0199-bbbb, which is not on this device',
          'split 0199-cccc needs txn 0199-aaaa, which is not on this device',
        ],
      ),
    );

    await importFrom(tester);

    // A deferred change is data that did *not* arrive. A bare number reads
    // exactly like a clean import, so each one is spelled out along with what
    // to do about it.
    expect(find.text('Deferred 2 — these did not arrive'), findsOneWidget);
    expect(
      find.textContaining('txn 0199-aaaa needs merchant 0199-bbbb'),
      findsOneWidget,
    );
    expect(
      find.textContaining('split 0199-cccc needs txn 0199-aaaa'),
      findsOneWidget,
    );
    expect(find.textContaining('Import that file if you still have it'),
        findsOneWidget);
  });

  testWidgets('kinds from a newer release are named, not swallowed',
      (tester) async {
    await pump(
      tester,
      importResult: applied(taken: 1, unknownKinds: ['budget_note']),
    );

    await importFrom(tester);

    expect(
      find.textContaining('This version does not understand: budget_note'),
      findsOneWidget,
    );
    expect(find.textContaining('update this one'), findsOneWidget);
  });

  testWidgets('a wrong phrase is reported without repeating the phrase',
      (tester) async {
    await pump(
      tester,
      importError:
          'the phrase does not match, or the file has been altered',
    );

    await importFrom(tester, phrase: 'hunter2 knows the ledger');

    expect(
      find.textContaining('the phrase does not match'),
      findsOneWidget,
    );
    // An error message is the last place a secret should turn up, because
    // errors are what get pasted into bug reports.
    expect(
      labels(tester).where((line) => line.contains('hunter2')),
      isEmpty,
    );
  });

  testWidgets('the phrase is obscured until it is asked for', (tester) async {
    await pump(tester);

    TextField field() =>
        tester.widget<TextField>(find.byKey(const Key('export-phrase')));
    expect(field().obscureText, isTrue);

    // Revealable on purpose: it is typed from memory on two machines, and a
    // box that cannot be checked turns one typo into "the phrase does not
    // match" on the other device an hour later.
    await tester.tap(find.byTooltip('Show the phrase').first);
    await tester.pumpAndSettle();
    expect(field().obscureText, isFalse);
  });

  testWidgets('open disagreements are counted where they can be seen',
      (tester) async {
    await pump(tester, openConflicts: 3);

    expect(
      find.text('3 disagreements waiting to be looked at'),
      findsOneWidget,
    );
    expect(find.textContaining('flagged in the Inbox'), findsOneWidget);
  });

  group('in a browser', () {
    testWidgets('there are no paths to type', (tester) async {
      await pump(tester, browser: true);

      // A page can neither write to a path nor read from one, so a field
      // asking for one would be a field that can only be wrong.
      expect(find.byKey(const Key('export-path')), findsNothing);
      expect(find.byKey(const Key('import-path')), findsNothing);
      expect(find.widgetWithText(FilledButton, 'Download'), findsOneWidget);
    });

    testWidgets('a download is not counted as sent until somebody says so',
        (tester) async {
      final calls = await pump(tester, browser: true);

      await tester.enterText(
          find.byKey(const Key('export-phrase')), 'seven rusty kettles');
      await tester.tap(find.widgetWithText(FilledButton, 'Download'));
      await tester.pumpAndSettle();

      // The browser was handed the file and says nothing about what became
      // of it. Marking it now would lose a cancelled download's contents for
      // good; not marking it costs one re-send.
      expect(calls.log, ['export', 'download']);
      expect(calls.downloaded.single, endsWith('.paccsync'));
      expect(find.textContaining('Downloaded pacc-'), findsOneWidget);
      expect(find.textContaining('for phone-7f2c'), findsOneWidget);

      await tester.tap(find.widgetWithText(OutlinedButton, 'Mark as sent'));
      await tester.pumpAndSettle();

      expect(calls.log, ['export', 'download', 'mark']);
      expect(calls.markedFor, [null]);
      expect(find.textContaining('Marked as sent'), findsOneWidget);
      expect(find.widgetWithText(OutlinedButton, 'Mark as sent'), findsNothing);
    });

    testWidgets('the first file of a pairing offers nothing to mark',
        (tester) async {
      final calls = await pump(tester, browser: true, peers: const []);

      await tester.enterText(
          find.byKey(const Key('export-phrase')), 'seven rusty kettles');
      await tester.tap(find.widgetWithText(FilledButton, 'Download'));
      await tester.pumpAndSettle();

      expect(calls.log, ['export', 'download']);
      expect(find.textContaining('first file of a pairing'), findsOneWidget);
      expect(find.widgetWithText(OutlinedButton, 'Mark as sent'), findsNothing);
    });

    testWidgets('import applies the file that was chosen, and names it',
        (tester) async {
      final calls = await pump(
        tester,
        browser: true,
        picked: PickedFile(
          name: 'from-phone.paccsync',
          bytes: Uint8List.fromList(const [9, 8, 7]),
        ),
        importResult: applied(taken: 4),
      );

      await tester.enterText(
          find.byKey(const Key('import-phrase')), 'seven rusty kettles');
      await tester.tap(find.widgetWithText(FilledButton, 'Choose file…'));
      await tester.pumpAndSettle();

      expect(calls.log, ['pick', 'import']);
      expect(calls.imported.single, [9, 8, 7]);
      expect(find.text('Read from-phone.paccsync.'), findsOneWidget);
      expect(find.text('Applied 4'), findsOneWidget);
    });

    testWidgets('closing the picker is not an error', (tester) async {
      final calls = await pump(tester, browser: true);

      await tester.enterText(
          find.byKey(const Key('import-phrase')), 'seven rusty kettles');
      await tester.tap(find.widgetWithText(FilledButton, 'Choose file…'));
      await tester.pumpAndSettle();

      expect(calls.log, ['pick']);
      expect(labels(tester).where((l) => l.startsWith('Read ')), isEmpty);
    });

    testWidgets('no phrase, no picker', (tester) async {
      final calls = await pump(tester, browser: true);

      await tester.tap(find.widgetWithText(FilledButton, 'Choose file…'));
      await tester.pumpAndSettle();

      // Asked for first because the picker is where the user's attention
      // goes next; a phrase box they have to come back to is one they miss.
      expect(calls.log, isEmpty);
      expect(find.textContaining('Enter the pairing phrase'), findsOneWidget);
    });
  });

  testWidgets('no disagreements, no banner', (tester) async {
    await pump(tester, openConflicts: 0);

    expect(find.textContaining('waiting to be looked at'), findsNothing);
  });
}
