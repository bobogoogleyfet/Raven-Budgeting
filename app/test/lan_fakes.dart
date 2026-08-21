/// Sockets that are not sockets, for tests that must not touch the network.
///
/// A unit test that bound a real port would depend on the machine running it:
/// its firewall, whether another copy of the suite already holds 47654, and
/// whether the box has a network at all. These stand in for the three
/// interfaces `lib/src/sync/links.dart` narrows `dart:io` down to, which is
/// the whole reason that narrowing exists.
library;

import 'dart:async';
import 'dart:typed_data';

import 'package:app/src/api.dart';
import 'package:app/src/sync/links.dart';

/// One end of a connection, with the wire under the test's control.
class FakeLink implements SyncLink {
  FakeLink({this.peer = '10.0.0.9:51000'});

  final StreamController<Uint8List> _incoming =
      StreamController<Uint8List>(sync: true);
  final BytesBuilder _outgoing = BytesBuilder();

  /// Whether the far end was told "nothing more from me" — the half-close the
  /// protocol treats as an acknowledgement, so a test can check it was sent at
  /// all and only after the work was done.
  bool finished = false;
  bool abandoned = false;
  int flushes = 0;

  @override
  final String peer;

  @override
  Stream<Uint8List> get incoming => _incoming.stream;

  @override
  void send(List<int> bytes) => _outgoing.add(bytes);

  @override
  Future<void> flush() async => flushes++;

  @override
  Future<void> finishSending() async => finished = true;

  @override
  void abandon() {
    abandoned = true;
    if (!_incoming.isClosed) _incoming.close();
  }

  /// Everything this end has written, as one buffer.
  Uint8List get sent => Uint8List.fromList(_outgoing.toBytes());

  /// Pretend the peer sent these bytes, in one chunk.
  void deliver(List<int> bytes) => _incoming.add(Uint8List.fromList(bytes));

  /// Pretend the peer closed its end.
  void hangUp() {
    if (!_incoming.isClosed) _incoming.close();
  }
}

/// Somewhere for fake connections to arrive.
class FakeListener implements SyncListener {
  final StreamController<SyncLink> _connections =
      StreamController<SyncLink>(sync: true);

  bool closed = false;

  @override
  int get port => 51234;

  @override
  Stream<SyncLink> get connections => _connections.stream;

  @override
  Future<void> close() async {
    closed = true;
    // Not awaited: a controller's `close` future completes when the done event
    // reaches a listener, and the service has already stopped listening by the
    // time it closes the listener down.
    if (!_connections.isClosed) unawaited(_connections.close());
  }

  /// Somebody connected.
  void arrive(SyncLink link) => _connections.add(link);
}

/// A broadcast socket that shouts into a list.
class FakeAnnouncer implements Announcer {
  final StreamController<Beam> _beams = StreamController<Beam>(sync: true);

  /// Every datagram this device broadcast, in order.
  final List<Uint8List> shouted = <Uint8List>[];
  bool closed = false;

  @override
  Stream<Beam> get beams => _beams.stream;

  @override
  void announce(List<int> bytes) => shouted.add(Uint8List.fromList(bytes));

  @override
  void close() {
    closed = true;
    if (!_beams.isClosed) _beams.close();
  }

  /// A datagram arrived from somewhere on the network.
  void hear(Beam beam) => _beams.add(beam);
}

/// What the fake bridge was asked to do, in the order it was asked.
///
/// Order is most of what these tests are about: a bundle recorded as delivered
/// before the peer acknowledged it is a bundle whose contents are never sent
/// again.
class BridgeCalls {
  final List<String> log = <String>[];

  /// The peer each `markDelivered` named, so a test can catch a bundle built
  /// for one device being written off against another.
  final List<String?> markedFor = <String?>[];

  /// Every set of bytes handed to `importBundle`.
  final List<Uint8List> merged = <Uint8List>[];
}

Applied applied({
  int taken = 0,
  int ignored = 0,
  int conflicts = 0,
  List<String> deferred = const <String>[],
  List<String> unknownKinds = const <String>[],
}) =>
    Applied(
      applied: taken,
      ignored: ignored,
      conflicts: conflicts,
      deferred: deferred,
      unknownKinds: unknownKinds,
    );
