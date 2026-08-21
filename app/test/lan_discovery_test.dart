import 'dart:convert';
import 'dart:typed_data';

import 'package:app/src/sync/discovery.dart';
import 'package:flutter_test/flutter_test.dart';

Uint8List datagram(Object? json) =>
    Uint8List.fromList(utf8.encode(jsonEncode(json)));

void main() {
  test('a beacon says who and where, and nothing else', () {
    final beacon = readBeacon(
      beaconBytes(device: 'phone-7f2c', port: 51234),
      ownDevice: 'laptop-91ab',
      from: '10.0.0.9',
    );

    expect(beacon, isNotNull);
    expect(beacon!.device, 'phone-7f2c');
    expect(beacon.port, 51234);
    // The address comes from the packet, not from the packet's contents: a
    // device that lied about its address would only be lying about where to
    // find itself.
    expect(beacon.host, '10.0.0.9');

    // Nothing secret is on the wire in the clear. The bundle is the only thing
    // that carries data, and it is sealed before this file ever sees it.
    final body = jsonDecode(utf8.decode(beaconBytes(device: 'a', port: 1)));
    expect((body as Map).keys, unorderedEquals(<String>['pacc', 'device', 'port']));
  });

  test('this device ignores its own broadcast', () {
    // Broadcasting to the subnet includes this machine, so every device hears
    // itself several times a minute. A device that treated that as a peer
    // would spend its life syncing with the database it is already using.
    expect(
      readBeacon(
        beaconBytes(device: 'laptop-91ab', port: 51234),
        ownDevice: 'laptop-91ab',
        from: '10.0.0.4',
      ),
      isNull,
    );
  });

  test('nothing on an open port is an error', () {
    for (final rubbish in <Uint8List>[
      Uint8List.fromList([0xff, 0xfe, 0x00, 0x01]), // not text at all
      Uint8List.fromList(utf8.encode('hello?')), // text, not JSON
      datagram(<String>['pacc', '1']), // JSON, not an object
      datagram(<String, Object>{'pacc': 2, 'device': 'x', 'port': 1}), // newer
      datagram(<String, Object>{'device': 'x', 'port': 1}), // not ours
      datagram(<String, Object>{'pacc': 1, 'port': 1}), // nameless
      datagram(<String, Object>{'pacc': 1, 'device': '', 'port': 1}),
      datagram(<String, Object>{'pacc': 1, 'device': 'x', 'port': 0}),
      datagram(<String, Object>{'pacc': 1, 'device': 'x', 'port': 70000}),
      datagram(<String, Object>{'pacc': 1, 'device': 'x', 'port': '51234'}),
    ]) {
      expect(
        readBeacon(rubbish, ownDevice: 'laptop-91ab', from: '10.0.0.9'),
        isNull,
        reason: 'a broadcast port hears everything, and must shrug at all of it',
      );
    }
  });

  test('a join request says who is joining, and nothing else', () {
    final asked = readJoinRequest(joinRequestBytes(device: 'phone-7f2c'));

    expect(asked, isNotNull);
    // The name the core keeps watermarks against, which is the only reason
    // this message exists: the device handing over its ledger records the
    // handover under it, and that record is what pairs the two.
    expect(asked!.device, 'phone-7f2c');

    final body = jsonDecode(utf8.decode(joinRequestBytes(device: 'a')));
    expect((body as Map).keys, unorderedEquals(<String>['pacc', 'join']));
  });

  test('a nameless device does not get to ask', () {
    // Refused here rather than by the far end, which would have to refuse it
    // after handing over its whole history.
    expect(() => joinRequestBytes(device: ''), throwsA(isA<Exception>()));
  });

  test('an ordinary bundle is not mistaken for a join request', () {
    for (final bundle in <Uint8List>[
      Uint8List(0), // an older release's join: no data, and no name either
      Uint8List.fromList(<int>[0x8f, 0x00, 0xff, 0x11]), // sealed bytes
      datagram(<String, Object>{'pacc': 1, 'device': 'x', 'port': 1}),
      // Over the cap, so never decoded at all: the length in front of a frame
      // is a stranger's, and it must not be able to buy a JSON parse of 60 MiB.
      Uint8List.fromList(
        utf8.encode(jsonEncode(<String, Object>{
          'pacc': 1,
          'join': 'x' * maxJoinRequestBytes,
        })),
      ),
    ]) {
      expect(readJoinRequest(bundle), isNull);
    }
  });

  test('a join request this end cannot read is refused, not guessed at', () {
    // Anything carrying `join` is claiming to be one of these. Unlike a
    // datagram on a broadcast port, it arrived on a connection that is waiting
    // for a ledger, and filling in what it left out is exactly what addressed
    // a handover to a device that did not exist.
    for (final broken in <Uint8List>[
      datagram(<String, Object?>{'pacc': 1, 'join': ''}),
      datagram(<String, Object?>{'pacc': 1, 'join': 7}),
      datagram(<String, Object?>{'pacc': 1, 'join': null}),
      datagram(<String, Object?>{'join': 'phone-7f2c'}),
      datagram(<String, Object?>{'pacc': 2, 'join': 'phone-7f2c'}),
    ]) {
      expect(() => readJoinRequest(broken), throwsA(isA<Exception>()));
    }
  });
}
