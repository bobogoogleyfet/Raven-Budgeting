import 'dart:typed_data';

import 'package:app/src/api.dart';
import 'package:app/src/screens/settings_screen.dart';
import 'package:app/src/sync/discovery.dart';
import 'package:app/src/sync/frames.dart';
import 'package:app/src/sync/lan_sync.dart';
import 'package:app/src/sync/links.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'lan_fakes.dart';

const String secret = 'seventeen purple herrings';
final Uint8List bundle = Uint8List.fromList(List<int>.filled(32, 5));

/// The section, wired to a service whose sockets are fakes.
class Section {
  Section({
    this.enabled = false,
    this.phrase = '',
    this.peers = const <String>['phone-7f2c'],
    this.result,
  });

  final bool enabled;
  final String phrase;
  final List<String> peers;
  final Applied? result;

  final FakeListener listener = FakeListener();
  final FakeAnnouncer announcer = FakeAnnouncer();
  final BridgeCalls calls = BridgeCalls();

  /// What was written to the settings table, so a test can check the switch
  /// actually saved something rather than only looking switched.
  final List<String> saved = <String>[];

  late final LanSync lan = LanSync(
    loadSettings: () async => LanSettings(enabled: enabled, phrase: phrase),
    saveSettings: ({required bool enabled, required String phrase}) async {
      saved.add('$enabled/${phrase.isEmpty ? "none" : "set"}');
    },
    loadStatus: () async => SyncStatus(
      thisDevice: 'laptop-91ab',
      peers: peers,
      openConflicts: 0,
    ),
    loadRefusal: () async => null,
    export: ({String? peer, required String phrase}) async => bundle,
    merge: ({required Uint8List bytes, required String phrase}) async =>
        result ?? applied(taken: 2),
    adopt: ({required Uint8List bytes, required String phrase}) async =>
        applied(taken: 9),
    markSent: ({
      String? peer,
      required String phrase,
      required Uint8List bytes,
    }) async =>
        calls.log.add('mark'),
    openListener: () async => listener,
    openLink: (host, port) async {
      final link = FakeLink(peer: '$host:$port');
      // Armed with its answer before it is handed over, so an exchange that
      // starts the moment a peer is heard from also finishes — a session left
      // waiting on a socket is a timer this test would be failed for leaving
      // behind.
      link.deliver(framed(bundle));
      link.hangUp();
      return link;
    },
    openAnnouncer: (port) async => announcer,
    announceEvery: const Duration(seconds: 3),
    settleFor: const Duration(seconds: 1),
    catchUpEvery: const Duration(seconds: 60),
  );

  Future<void> pump(WidgetTester tester) async {
    // Tall enough that the whole section lays out: an overflowing Column fails
    // the test with a layout error rather than with the thing it is about.
    tester.view.physicalSize = const Size(1000, 3000);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);

    await lan.load();
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: SingleChildScrollView(child: LanSyncSection(lan: lan)),
        ),
      ),
    );
    await tester.pump();
  }

  /// A beacon from another device.
  void heard(String device) => announcer.hear(
        Beam(data: beaconBytes(device: device, port: 6000), from: '10.0.0.9'),
      );

  /// Somebody connected and sent a bundle.
  void arrives() {
    final link = FakeLink();
    listener.arrive(link);
    link.deliver(framed(bundle));
    link.hangUp();
  }

  /// Take the section down before the service, so nothing is listening to a
  /// notifier that has been disposed — and stop the timers, which a widget
  /// test counts as a leak.
  Future<void> close(WidgetTester tester) async {
    await tester.pumpWidget(const SizedBox());
    await lan.dispose();
  }
}

void main() {
  testWidgets('it is off until somebody turns it on, and says why it will not',
      (tester) async {
    final section = Section();
    await section.pump(tester);

    expect(tester.widget<SwitchListTile>(find.byKey(const Key('lan-switch'))).value,
        isFalse);
    expect(find.textContaining('announces nothing'), findsOneWidget);

    // No phrase typed and none stored: there is nothing to seal a bundle with,
    // and turning on regardless would look like it was working.
    await tester.tap(find.byKey(const Key('lan-switch')));
    await tester.pumpAndSettle();

    expect(section.saved, isEmpty);
    expect(find.textContaining('Enter the pairing phrase first'), findsOneWidget);

    await section.close(tester);
  });

  testWidgets('a phrase typed once turns it on and is not shown again',
      (tester) async {
    final section = Section();
    await section.pump(tester);

    await tester.enterText(find.byKey(const Key('lan-phrase')), secret);
    await tester.pump();
    await tester.tap(find.byKey(const Key('lan-switch')));
    await tester.pumpAndSettle();

    expect(section.saved, contains('true/set'));
    expect(
      tester.widget<SwitchListTile>(find.byKey(const Key('lan-switch'))).value,
      isTrue,
    );
    // Never repeated back to the screen: a stored secret on a screen is a
    // stored secret in the next screenshot.
    expect(find.text(secret), findsNothing);

    await section.close(tester);
  });

  testWidgets('a stored phrase is not filled back into the field',
      (tester) async {
    final section = Section(enabled: true, phrase: secret);
    await section.pump(tester);

    expect(find.text(secret), findsNothing);
    expect(find.text('New pairing phrase'), findsOneWidget);
    expect(find.textContaining('is already saved and is not shown'),
        findsOneWidget);

    await section.close(tester);
  });

  testWidgets('devices on the network are listed', (tester) async {
    final section = Section(enabled: true, phrase: secret);
    await section.pump(tester);

    section.heard('phone-7f2c');
    await tester.pump();

    expect(find.byKey(const Key('lan-peer-phone-7f2c')), findsOneWidget);
    expect(find.text('10.0.0.9:6000'), findsOneWidget);

    await section.close(tester);
  });

  testWidgets('an unpaired device is offered the one destructive step',
      (tester) async {
    final section =
        Section(enabled: true, phrase: secret, peers: const <String>[]);
    await section.pump(tester);

    section.heard('tablet-0000');
    await tester.pump();

    expect(find.text('Not paired yet'), findsOneWidget);
    expect(find.text('Join'), findsOneWidget);

    // And it asks before doing it: joining throws away everything on this
    // device, which is not something a mis-tap should be able to do.
    await tester.tap(find.text('Join'));
    await tester.pumpAndSettle();
    expect(find.text('Join tablet-0000?'), findsOneWidget);
    expect(find.textContaining('everything currently here is replaced'),
        findsOneWidget);

    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();

    await section.close(tester);
  });

  testWidgets('a paired device is not offered it', (tester) async {
    final section = Section(enabled: true, phrase: secret);
    await section.pump(tester);
    section.heard('phone-7f2c');
    await tester.pump();

    expect(find.text('Join'), findsNothing);
    expect(find.text('Not paired yet'), findsNothing);

    await section.close(tester);
  });

  testWidgets('what did not arrive is spelled out, not counted', (tester) async {
    final section = Section(
      enabled: true,
      phrase: secret,
      result: applied(
        taken: 2,
        deferred: <String>['txn 9f21 needs category 4b0c, which is not on this device'],
      ),
    );
    await section.pump(tester);

    section.arrives();
    await tester.pumpAndSettle();

    // A deferred change means data did *not* arrive, and a bare number reads
    // exactly like a clean import — so the line itself is on screen.
    expect(find.textContaining('Deferred 1'), findsOneWidget);
    expect(find.textContaining('needs category 4b0c'), findsOneWidget);

    await section.close(tester);
  });

  testWidgets('a clean exchange says so without a wall of counts',
      (tester) async {
    final section = Section(enabled: true, phrase: secret);
    await section.pump(tester);

    section.arrives();
    await tester.pumpAndSettle();

    expect(find.textContaining('Synced with'), findsOneWidget);
    expect(find.textContaining('Deferred'), findsNothing);

    await section.close(tester);
  });
}
