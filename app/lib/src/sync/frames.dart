/// Putting a bundle on a socket, and getting one back off it.
///
/// TCP is a stream of bytes with no notion of where one message stops, so a
/// reader that simply drains until the connection closes cannot tell "the
/// bundle ended" from "the Wi-Fi dropped halfway through" — and would apply a
/// truncated file to a ledger. Four bytes of big-endian length in front of
/// every payload turns the stream back into messages, and makes a short read
/// an error rather than a silent half-import.
library;

import 'dart:async';
import 'dart:typed_data';

/// The largest payload this end will read.
///
/// A bundle is a household's ledger; 64 MiB is orders of magnitude more than
/// one will ever be, and the number exists for the opposite case. The length
/// arrives from a socket **anyone on the Wi-Fi can connect to**, so a claimed
/// length is a stranger's instruction to allocate memory. It is checked before
/// a single byte is reserved: four bytes saying `0xFFFFFFFF` must cost a
/// refusal, not four gigabytes.
const int maxFrameBytes = 64 * 1024 * 1024;

/// The connection said something this end will not act on.
///
/// Its own type so a session can tell a protocol violation — which means close
/// this socket and carry on serving — apart from a bridge error, which means
/// the phrase was wrong or the ledger refused the change.
class SyncProtocolException implements Exception {
  const SyncProtocolException(this.message);

  final String message;

  @override
  String toString() => message;
}

/// One payload, ready to write: its length, then it.
///
/// Refuses to *send* something over the cap as well as to receive it. The far
/// end would refuse it anyway, and finding that out after uploading 80 MiB
/// over Wi-Fi is a worse way to learn it.
Uint8List framed(Uint8List payload) {
  if (payload.length > maxFrameBytes) {
    throw SyncProtocolException(
      'a sync bundle of ${payload.length} bytes is larger than the '
      '$maxFrameBytes-byte limit either end will read',
    );
  }
  final frame = Uint8List(4 + payload.length);
  ByteData.view(frame.buffer).setUint32(0, payload.length);
  frame.setRange(4, frame.length, payload);
  return frame;
}

/// Reads whole payloads off a stream of arbitrary chunks.
///
/// Chunk boundaries are the network's business and never the message's: a
/// bundle can arrive as one chunk, as a thousand, or with the next frame's
/// first bytes glued to its tail. This holds the leftovers so a session can
/// ask for messages and never think about any of that.
class FrameReader {
  FrameReader(Stream<Uint8List> chunks) : _chunks = StreamIterator(chunks);

  final StreamIterator<Uint8List> _chunks;

  /// Bytes that have arrived and not yet been handed out, oldest first. Kept
  /// as the chunks they came in rather than concatenated on every arrival —
  /// joining a growing buffer per packet is quadratic, and a bundle is exactly
  /// the size where that starts to show.
  final List<Uint8List> _held = <Uint8List>[];
  int _heldBytes = 0;

  /// The next payload, or a throw if the connection cannot supply one.
  ///
  /// An empty payload is a legitimate message meaning "nothing from me", not
  /// an end of stream — a device that has only connected to *ask* for a bundle
  /// sends one, and it must be answerable rather than mistaken for a hang-up.
  Future<Uint8List> next() async {
    await _gather(4, 'a frame header');
    final header = _take(4);
    final length = ByteData.view(header.buffer, header.offsetInBytes).getUint32(0);
    // Before the allocation, not after: this is the whole reason the cap
    // exists, and a check that runs once the memory is reserved has already
    // let the attack succeed.
    if (length > maxFrameBytes) {
      throw SyncProtocolException(
        'the peer announced a $length-byte bundle, over the $maxFrameBytes-byte '
        'limit; nothing was read',
      );
    }
    await _gather(length, 'a $length-byte bundle');
    return _take(length);
  }

  /// Waits for the peer to hang up, and drains anything it says on the way.
  ///
  /// This is the only acknowledgement the protocol has. The far end closes its
  /// end *after* it has applied what this one sent, so a clean end of stream is
  /// evidence the bundle landed — which is what a watermark may be advanced on
  /// and nothing weaker.
  Future<void> untilClosed() async {
    while (await _chunks.moveNext()) {
      // Trailing bytes from a peer that has already said its piece are not
      // worth failing over; they are simply not part of any message this side
      // asked for.
    }
  }

  /// Stop reading, and let the underlying subscription go.
  Future<void> cancel() => _chunks.cancel();

  /// Pull chunks until [wanted] bytes are in hand.
  Future<void> _gather(int wanted, String what) async {
    while (_heldBytes < wanted) {
      if (!await _chunks.moveNext()) {
        throw SyncProtocolException(
          'the connection ended while waiting for $what — $_heldBytes of '
          '$wanted bytes arrived',
        );
      }
      final chunk = _chunks.current;
      if (chunk.isEmpty) continue;
      _held.add(chunk);
      _heldBytes += chunk.length;
    }
  }

  /// Hand out the first [count] held bytes, keeping the remainder.
  Uint8List _take(int count) {
    final out = Uint8List(count);
    int filled = 0;
    while (filled < count) {
      final chunk = _held.first;
      final wanted = count - filled;
      if (chunk.length <= wanted) {
        out.setRange(filled, filled + chunk.length, chunk);
        filled += chunk.length;
        _held.removeAt(0);
      } else {
        out.setRange(filled, count, chunk);
        // A view, not a copy: the leftover is the tail of a chunk that is
        // usually about to be consumed by the very next read.
        _held[0] = Uint8List.sublistView(chunk, wanted);
        filled = count;
      }
    }
    _heldBytes -= count;
    return out;
  }
}
