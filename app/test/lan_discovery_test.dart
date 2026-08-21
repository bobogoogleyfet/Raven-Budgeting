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
}
