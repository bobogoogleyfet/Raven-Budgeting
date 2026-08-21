import 'dart:typed_data';

import 'package:app/src/api.dart';
import 'package:app/src/sync/discovery.dart';
import 'package:app/src/sync/frames.dart';
import 'package:app/src/sync/lan_sync.dart';
import 'package:app/src/sync/links.dart';
import 'package:flutter_test/flutter_test.dart';

import 'lan_fakes.dart';

/// The phrase, which must never turn up in anything the service says.
const String secret = 'seventeen purple herrings';

/// What this device would export, so a test can tell it apart from what
/// arrives.
final Uint8List ourBundle = Uint8List.fromList(List<int>.filled(64, 1));

/// What the other device sends.
final Uint8List theirBundle = Uint8List.fromList(List<int>.filled(48, 2));

/// One service, its fake sockets, and a record of every bridge call it made.
class Bench {
  Bench({
    this.peers = const <String>['phone-7f2c'],
    this.mergeResult,
    this.mergeErrors = 0,
    this.refusal,
    this.reply,
  });

  /// The devices the core says this one already syncs with. The whole of the
  /// safety story turns on this: a device with no peers merges nothing.
  List<String> peers;

  final Applied? mergeResult;

  /// How many of the first imports should fail — a wrong phrase, a truncated
  /// file — before one succeeds.
  int mergeErrors;

  final String? refusal;

  /// What a dialled peer replies with. Null means "the usual bundle".
  final Uint8List? reply;

  final BridgeCalls calls = BridgeCalls();
  final FakeListener listener = FakeListener();
  final FakeAnnouncer announcer = FakeAnnouncer();

  /// Every connection this device opened, and what it was aimed at.
  final List<String> dialled = <String>[];
  final List<FakeLink> links = <FakeLink>[];

  /// The peer each export was addressed to.
  final List<String?> exportedFor = <String?>[];

  late final LanSync lan = LanSync(
    loadSettings: () async =>
        const LanSettings(enabled: true, phrase: secret),
    saveSettings: ({required bool enabled, required String phrase}) async {},
    loadStatus: () async => SyncStatus(
      thisDevice: 'laptop-91ab',
      peers: peers,
      openConflicts: 0,
    ),
    loadRefusal: () async => refusal,
    export: ({String? peer, required String phrase}) async {
      expect(phrase, secret);
      calls.log.add('export');
      exportedFor.add(peer);
      return ourBundle;
    },
    merge: ({required Uint8List bytes, required String phrase}) async {
      expect(phrase, secret);
      calls.log.add('import');
      calls.merged.add(bytes);
      if (mergeErrors > 0) {
        mergeErrors--;
        throw Exception('the bundle could not be opened — the phrase does not match');
      }
      return mergeResult ?? applied(taken: 3, ignored: 1);
    },
    adopt: ({required Uint8List bytes, required String phrase}) async {
      calls.log.add('join');
      return applied(taken: 40);
    },
    markSent: ({
      String? peer,
      required String phrase,
      required Uint8List bytes,
    }) async {
      calls.log.add('mark');
      calls.markedFor.add(peer);
    },
    openListener: () async => listener,
    openLink: (host, port) async {
      dialled.add('$host:$port');
      final link = FakeLink(peer: '$host:$port');
      // Armed before it is handed over: a `StreamController` holds events
      // until somebody listens, so the answer is waiting the moment the
      // service asks for it.
      link.deliver(framed(reply ?? theirBundle));
      link.hangUp();
      links.add(link);
      return link;
    },
    openAnnouncer: (port) async => announcer,
    announceEvery: const Duration(milliseconds: 500),
    settleFor: const Duration(milliseconds: 20),
    catchUpEvery: const Duration(seconds: 30),
    patience: const Duration(seconds: 2),
  );

  /// Start the service and let its opening exchange finish.
  Future<void> start() async {
    await lan.load();
    await settle();
  }

  /// A beacon from another device on the network.
  void heard({String device = 'phone-7f2c', String host = '10.0.0.9', int port = 6000}) {
    announcer.hear(
      Beam(data: beaconBytes(device: device, port: port), from: host),
    );
  }

  /// Somebody connected to this device and sent [bytes].
  FakeLink connectionSending(Uint8List bytes) {
    final link = FakeLink();
    listener.arrive(link);
    link.deliver(framed(bytes));
    link.hangUp();
    return link;
  }

  /// Let every queued exchange run to the end.
  Future<void> settle() async {
    for (int i = 0; i < 4; i++) {
      await pumpEventQueue();
    }
  }

  /// The payloads this device wrote to [link], unframed.
  Future<List<Uint8List>> written(FakeLink link) async {
    final reader = FrameReader(Stream<Uint8List>.value(link.sent));
    final out = <Uint8List>[];
    try {
      while (true) {
        out.add(await reader.next());
      }
    } on SyncProtocolException {
      // The end of what was written, which is the only way this loop stops.
    }
    return out;
  }

  LanSyncState get state => lan.state.value;

  Future<void> close() => lan.dispose();
}

void main() {
  test('a session applies what arrived and says what it did', () async {
    final bench = Bench();
    addTearDown(bench.close);
    await bench.start();

    final link = bench.connectionSending(theirBundle);
    await bench.settle();

    expect(bench.calls.merged.single, theirBundle);
    // Import, then reply, then record the reply as delivered — and only once
    // the far end has hung up, which it does after applying what it was sent.
    expect(bench.calls.log, ['import', 'export', 'mark']);
    expect(await bench.written(link), [ourBundle]);
    expect(bench.state.lastFailed, isFalse);
    expect(bench.state.lastResult, contains('3 applied'));
    expect(bench.state.lastResult, contains('1 already here'));
  });

  test('a wrong phrase is reported without repeating it, and the listener lives',
      () async {
    final bench = Bench(mergeErrors: 1);
    addTearDown(bench.close);
    await bench.start();

    final bad = bench.connectionSending(theirBundle);
    await bench.settle();

    expect(bench.state.lastFailed, isTrue);
    expect(bench.state.lastResult, contains('the phrase does not match'));
    // An error message is the last place a secret should turn up, because
    // errors are what people paste into bug reports.
    expect(bench.state.lastResult, isNot(contains(secret)));
    // Nothing was recorded as delivered on a session that failed: doing so
    // would mean those changes were never sent again.
    expect(bench.calls.log, isNot(contains('mark')));
    expect(bad.abandoned, isTrue);

    // One bad connection must cost one socket and nothing else.
    bench.connectionSending(theirBundle);
    await bench.settle();
    expect(bench.state.lastFailed, isFalse);
    expect(bench.calls.log, contains('mark'));
  });

  test('a device with no peers merges nothing a stranger sends', () async {
    // Two devices that have each been opened have each seeded their own chart
    // of accounts, and merging those leaves two of every category. Pairing is
    // a decision somebody makes, so until it has been made this answers and
    // applies nothing.
    final bench = Bench(peers: const <String>[]);
    addTearDown(bench.close);
    await bench.start();

    bench.connectionSending(theirBundle);
    await bench.settle();

    expect(bench.calls.log, isNot(contains('import')));
    expect(bench.calls.log, isNot(contains('mark')));
    expect(bench.state.lastResult, contains('not paired'));
    expect(bench.state.paired, isFalse);
  });

  test('a peer that appears is exchanged with, and its own name is used',
      () async {
    final bench = Bench();
    addTearDown(bench.close);
    await bench.start();

    bench.heard(host: '10.0.0.9', port: 6000);
    await bench.settle();

    expect(bench.dialled, ['10.0.0.9:6000']);
    expect(await bench.written(bench.links.single), [ourBundle]);
    expect(bench.calls.merged.single, theirBundle);
    // Addressed to the device actually on the other end, not to "the peer":
    // with three devices in a household, a bundle built for one and written
    // off against another convinces this device it sent changes nobody got.
    expect(bench.exportedFor, contains('phone-7f2c'));
    expect(bench.calls.markedFor, ['phone-7f2c']);
    // The half-close is this end's receipt, and it goes out only after the
    // peer's bundle has been applied here.
    expect(bench.links.single.finished, isTrue);
  });

  test('this device does not sync with itself', () async {
    final bench = Bench();
    addTearDown(bench.close);
    await bench.start();

    // Broadcasting to the subnet includes this machine, so this arrives
    // several times a minute for as long as the feature is on.
    bench.heard(device: 'laptop-91ab');
    await bench.settle();

    expect(bench.dialled, isEmpty);
    expect(bench.state.peers, isEmpty);
  });

  test('a device nobody has paired with is not dialled', () async {
    final bench = Bench();
    addTearDown(bench.close);
    await bench.start();

    bench.heard(device: 'tablet-0000');
    await bench.settle();

    // Listed, so a person can choose to join it. Not merged with, because
    // its ids are its own.
    expect(bench.state.peers.map((p) => p.device), ['tablet-0000']);
    expect(bench.dialled, isEmpty);
  });

  test('a burst of local edits is one exchange', () async {
    final bench = Bench();
    addTearDown(bench.close);
    await bench.start();
    bench.heard();
    await bench.settle();
    final before = bench.dialled.length;

    // "Save & New" three times is one thing that happened.
    bench.lan.nudge();
    bench.lan.nudge();
    bench.lan.nudge();
    await Future<void>.delayed(const Duration(milliseconds: 60));
    await bench.settle();

    expect(bench.dialled.length - before, 1);
  });

  test('a pull is answered in full and never written off as delivered',
      () async {
    final bench = Bench();
    addTearDown(bench.close);
    await bench.start();

    // An empty frame is a device about to adopt this one's data saying
    // "nothing from me" — it is about to throw its own copy away.
    final link = bench.connectionSending(Uint8List(0));
    await bench.settle();

    expect(bench.calls.log, isNot(contains('import')));
    expect(await bench.written(link), [ourBundle]);
    // Addressed to a name no device has, so the core finds no watermark for it
    // and hands over the whole history rather than the tail of it.
    expect(bench.exportedFor.last, '(joining)');
    expect(bench.calls.log, isNot(contains('mark')));
  });

  test('joining adopts the peer’s data instead of merging it', () async {
    final bench = Bench(peers: const <String>[]);
    addTearDown(bench.close);
    await bench.start();
    bench.heard(device: 'tablet-0000');
    await bench.settle();

    await bench.lan.joinPeer(bench.state.peers.single);
    await bench.settle();

    expect(bench.calls.log, contains('join'));
    expect(bench.calls.log, isNot(contains('import')));
    // Nothing of this device's own goes over first: handing its chart of
    // accounts across seconds before discarding it would leave the other one
    // holding a duplicate of everything.
    expect(await bench.written(bench.links.single), [isEmpty]);
    expect(bench.state.lastResult, contains('Joined tablet-0000'));
  });

  test('a device holding work of its own is told why it cannot join', () async {
    final bench = Bench(
      peers: const <String>[],
      refusal: 'this device already has 12 expense(s) of its own',
    );
    addTearDown(bench.close);
    await bench.start();
    bench.heard(device: 'tablet-0000');
    await bench.settle();

    await bench.lan.joinPeer(bench.state.peers.single);
    await bench.settle();

    expect(bench.calls.log, isNot(contains('join')));
    expect(bench.state.lastFailed, isTrue);
    expect(bench.state.lastResult, contains('12 expense(s)'));
  });

  test('turning it off stops announcing and answering', () async {
    final bench = Bench();
    addTearDown(bench.close);
    await bench.start();
    expect(bench.announcer.shouted, isNotEmpty);

    await bench.lan.turnOff();
    await bench.settle();

    expect(bench.announcer.closed, isTrue);
    expect(bench.listener.closed, isTrue);
    expect(bench.state.listening, isFalse);
    expect(bench.state.peers, isEmpty);
  });

  test('what it broadcasts is its name and the port it is listening on',
      () async {
    final bench = Bench();
    addTearDown(bench.close);
    await bench.start();

    final beacon = readBeacon(
      bench.announcer.shouted.first,
      ownDevice: 'somebody-else',
      from: '10.0.0.4',
    );
    expect(beacon!.device, 'laptop-91ab');
    expect(beacon.port, bench.listener.port);
  });
}
