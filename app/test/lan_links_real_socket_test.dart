// The only tests that open real sockets.
//
// Everything else in the LAN suite runs against fakes, which is right for the
// protocol logic — but it leaves `links.dart`, the one file that actually
// touches `dart:io`, covered by nothing. A wrong adapter there passes every
// fake-based test and fails only on a real network, which is the most
// expensive place to find out.
//
// These bind to loopback, so they are cheap and need no network.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:app/src/sync/links.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('a real listener and a real link carry bytes both ways', () async {
    final listener = await openSocketListener();
    addTearDown(() async => listener.close());
    expect(listener.port, greaterThan(0), reason: 'an ephemeral port was bound');

    // The server half: echo whatever arrives, with a marker, then finish.
    final served = Completer<List<int>>();
    final sub = listener.connections.listen((link) async {
      final seen = <int>[];
      await for (final chunk in link.incoming) {
        seen.addAll(chunk);
      }
      served.complete(seen);
      link.send(utf8.encode('pong'));
      await link.flush();
      await link.finishSending();
    });
    addTearDown(sub.cancel);

    final client = await openSocketLink('127.0.0.1', listener.port);
    client.send(utf8.encode('ping'));
    await client.flush();
    // Half-close: the server's `await for` only ends when the client stops
    // sending, which is exactly how the real session signals "that is all".
    await client.finishSending();

    expect(utf8.decode(await served.future), 'ping');
    final back = <int>[];
    await for (final chunk in client.incoming) {
      back.addAll(chunk);
    }
    expect(utf8.decode(back), 'pong', reason: 'the reply must survive the half-close');
  });

  test('abandoning a link does not take the listener down with it', () async {
    // One bad peer must not stop this device syncing with anybody else.
    final listener = await openSocketListener();
    addTearDown(() async => listener.close());

    final arrived = <SyncLink>[];
    final sub = listener.connections.listen(arrived.add);
    addTearDown(sub.cancel);

    final rude = await openSocketLink('127.0.0.1', listener.port);
    rude.abandon();
    await Future<void>.delayed(const Duration(milliseconds: 100));

    // The listener is still serving: a second client connects and is seen.
    final polite = await openSocketLink('127.0.0.1', listener.port);
    await Future<void>.delayed(const Duration(milliseconds: 100));
    polite.abandon();

    expect(arrived.length, greaterThanOrEqualTo(1),
        reason: 'the listener survived a client that hung up mid-handshake');
  });

  test('a broadcast socket binds and sends without throwing', () async {
    // Binding with reuseAddress/reusePort and enabling broadcast is the part
    // that differs per platform; a wrong flag throws here and nowhere else.
    final announcer = await openBroadcastSocket(47655);
    addTearDown(announcer.close);

    expect(
      () => announcer.announce(utf8.encode('{"pacc":1}')),
      returnsNormally,
    );
  });

  test('a datagram sent to the bound port is received', () async {
    // Proves the receive path, not just the send path. Loopback rather than
    // 255.255.255.255 so this does not depend on a network being present.
    final announcer = await openBroadcastSocket(47656);
    addTearDown(announcer.close);

    final heard = Completer<Uint8List>();
    final sub = announcer.beams.listen((beam) {
      if (!heard.isCompleted) heard.complete(beam.data);
    });
    addTearDown(sub.cancel);

    announcer.announce(utf8.encode('{"pacc":1,"device":"x","port":1}'));

    final bytes = await heard.future.timeout(
      const Duration(seconds: 5),
      onTimeout: () => Uint8List(0),
    );
    expect(
      utf8.decode(bytes),
      contains('"pacc":1'),
      reason: 'the announcer must hear datagrams on the port it bound',
    );
  });
}
