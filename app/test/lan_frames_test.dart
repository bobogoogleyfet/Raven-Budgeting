import 'dart:async';
import 'dart:typed_data';

import 'package:app/src/sync/frames.dart';
import 'package:flutter_test/flutter_test.dart';

/// A stream of exactly these chunks, then an end.
Stream<Uint8List> chunks(List<List<int>> parts) => Stream<Uint8List>.fromIterable(
      parts.map(Uint8List.fromList),
    );

void main() {
  test('a payload survives the trip through a frame', () async {
    final payload = Uint8List.fromList(List<int>.generate(5000, (i) => i % 256));
    final reader = FrameReader(chunks([framed(payload)]));

    expect(await reader.next(), payload);
  });

  test('chunk boundaries are not message boundaries', () async {
    final payload = Uint8List.fromList(List<int>.filled(300, 9));
    final frame = framed(payload);
    // Split across the length header itself, which is the case a reader that
    // assumed a header arrives whole would get wrong.
    final reader = FrameReader(
      chunks([
        frame.sublist(0, 2),
        frame.sublist(2, 7),
        frame.sublist(7),
      ]),
    );

    expect(await reader.next(), payload);
  });

  test('two frames on one connection do not run into each other', () async {
    final first = Uint8List.fromList([1, 2, 3]);
    final second = Uint8List.fromList([4, 5]);
    final reader = FrameReader(
      chunks([
        [...framed(first), ...framed(second)],
      ]),
    );

    expect(await reader.next(), first);
    expect(await reader.next(), second);
  });

  test('an empty frame is a message, not an ending', () async {
    // "Nothing from me" is what a device about to adopt another's data sends,
    // so it has to be readable rather than look like a hang-up.
    final reader = FrameReader(chunks([framed(Uint8List(0))]));

    expect(await reader.next(), isEmpty);
  });

  test('an oversized length is refused before anything is allocated', () async {
    // Four bytes of header claiming most of a gigabyte, and nothing behind
    // them. A reader that trusted the number would try to reserve it.
    final header = Uint8List(4);
    ByteData.view(header.buffer).setUint32(0, maxFrameBytes + 1);
    final reader = FrameReader(chunks([header]));

    await expectLater(
      reader.next(),
      throwsA(
        isA<SyncProtocolException>().having(
          (e) => e.message,
          'message',
          contains('over the'),
        ),
      ),
    );
  });

  test('an oversized payload is refused before it is sent', () {
    expect(
      () => framed(Uint8List(maxFrameBytes + 1)),
      throwsA(isA<SyncProtocolException>()),
    );
  });

  test('a connection that dies mid-bundle is an error, not a short bundle', () async {
    final payload = Uint8List.fromList(List<int>.filled(100, 7));
    final frame = framed(payload);
    final reader = FrameReader(chunks([frame.sublist(0, 40)]));

    await expectLater(
      reader.next(),
      throwsA(
        isA<SyncProtocolException>().having(
          (e) => e.message,
          'message',
          contains('ended while waiting'),
        ),
      ),
    );
  });

  test('waiting for the hang-up ends when the peer hangs up', () async {
    final wire = StreamController<Uint8List>();
    final reader = FrameReader(wire.stream);

    final waiting = reader.untilClosed();
    wire.add(Uint8List.fromList([1, 2, 3]));
    unawaited(wire.close());

    await expectLater(waiting, completes);
  });
}
