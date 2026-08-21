/// How two devices on one Wi-Fi find each other.
///
/// There is no server to register with and no name to type, so each device
/// simply says what it is every few seconds and listens for anyone else doing
/// the same. A shout rather than a query: a device that has just woken up
/// wants to be found without knowing who to ask, and the answer has to survive
/// the other device having been asleep when it asked.
///
/// Nothing here is trusted and nothing here is secret. A beacon says only "a
/// copy of this app is at this address, on this port" — everything that
/// matters is inside the sealed bundle a connection then carries, and a
/// stranger who forges a beacon achieves nothing beyond being connected to and
/// failing to produce a bundle the phrase will open.
///
/// The protocol's other message in the clear is here too, for the same
/// reasons: a *join request*, which a device sends in place of a bundle when
/// it is about to adopt another's data. It says who is asking and nothing
/// else, and like a beacon it is refused rather than guessed at when it does
/// not say that much.
library;

import 'dart:convert';
import 'dart:typed_data';

import 'frames.dart';

/// The one fixed port in the design.
///
/// Discovery has to be fixed — a port nobody agreed on in advance cannot be
/// listened on — but the *transfer* port is not, and is carried in the beacon
/// precisely so this is the only number two devices must share.
const int discoveryPort = 47654;

/// The shape of the datagram, so a later release can change it and be ignored
/// rather than misread by this one.
const int beaconVersion = 1;

/// Somebody else running this app, as of a moment ago.
class Beacon {
  const Beacon({required this.device, required this.host, required this.port});

  /// The peer's name for its *database*, the same name the core's watermarks
  /// are kept against — which is why a bundle can be addressed to it.
  final String device;

  final String host;

  /// Where that device is listening for an exchange. Not the port the datagram
  /// came from.
  final int port;
}

/// This device's beacon, ready to broadcast.
Uint8List beaconBytes({required String device, required int port}) =>
    Uint8List.fromList(
      utf8.encode(
        jsonEncode(<String, Object>{
          'pacc': beaconVersion,
          'device': device,
          'port': port,
        }),
      ),
    );

/// Read a datagram, or `null` if there is nothing here worth acting on.
///
/// Returns rather than throws for everything malformed. This is a port on an
/// open network: the datagrams that arrive include other software's, a port
/// scanner's, and a future version of this app's, and none of those is an
/// error a user should ever be shown — the only correct response is to ignore
/// them and keep listening.
///
/// [ownDevice] is filtered out because broadcasting to the subnet includes
/// this machine: every device receives its own beacon, and a device that
/// treated itself as a peer would spend its life exchanging bundles with the
/// database it is already using.
Beacon? readBeacon(
  Uint8List datagram, {
  required String ownDevice,
  required String from,
}) {
  final Object? parsed;
  try {
    parsed = jsonDecode(utf8.decode(datagram));
  } catch (_) {
    // Not ours. Anything at all may arrive on a broadcast port.
    return null;
  }
  if (parsed is! Map) return null;
  if (parsed['pacc'] != beaconVersion) return null;

  final Object? device = parsed['device'];
  if (device is! String || device.isEmpty || device == ownDevice) return null;

  final Object? port = parsed['port'];
  // A port outside the range is not a port, and 0 means "the OS will pick one"
  // — never something to connect to.
  if (port is! int || port <= 0 || port > 65535) return null;

  return Beacon(device: device, host: from, port: port);
}

/// The shape of a join request, versioned apart from the beacon: they travel
/// on different transports and answer different questions, and a release that
/// needs to change one has no reason to be held up by the other.
const int joinVersion = 1;

/// The largest frame that could be a join request.
///
/// A device name is a uuid, so a real request is well under a hundred bytes.
/// The cap is not about those: the length in front of a frame is chosen by
/// whoever connected, and running a stranger's 60 MiB through a JSON decoder
/// to find out it was never a control frame is work a stranger must not be
/// able to ask for. Anything over this is a bundle, and is left to the core.
const int maxJoinRequestBytes = 512;

/// A device opening a connection to say it is about to adopt this one's data.
///
/// What a *Join* sends in place of a bundle, and it carries exactly one thing:
/// the joiner's name for its database. That name is the point. The device
/// answering is about to hand over its entire history, and a handover it
/// cannot attribute is one it cannot record — no peer row, no watermark — so
/// pairing would come out one-sided, working in the direction that was set up
/// and silently refusing everything sent back the other way.
class JoinRequest {
  const JoinRequest({required this.device});

  /// The joiner's name for its *database*, the same name the core keeps
  /// watermarks against — which is why the reply can be addressed to it.
  final String device;
}

/// This device's join request, ready to send in place of a bundle.
///
/// Refuses a nameless one rather than sending it. An unattributable request is
/// precisely what the far end cannot write down, and failing here is far
/// better than failing after the other device has handed over its ledger.
Uint8List joinRequestBytes({required String device}) {
  if (device.isEmpty) {
    throw const SyncProtocolException(
      'this device has no name for its database yet, and a join request that '
      'does not say who is joining is one the other device cannot record',
    );
  }
  return Uint8List.fromList(
    utf8.encode(
      jsonEncode(<String, Object>{'pacc': joinVersion, 'join': device}),
    ),
  );
}

/// Read a connection's opening frame as a join request.
///
/// `null` means "not one of these": an ordinary exchange opens with a sealed
/// bundle, which is ciphertext and so is never a JSON object carrying `join`.
/// Such a frame is passed on to the core exactly as before.
///
/// A frame that *does* carry `join` is claiming to be a request, and from
/// there anything malformed throws rather than returning. Unlike a beacon — a
/// datagram on an open broadcast port, where the only sane response to rubbish
/// is to ignore it — this arrived on a connection that is waiting for a
/// ledger. Filling in what it left out with a guess is what addressed the
/// handover to a name no device had, and left pairing one-sided.
JoinRequest? readJoinRequest(Uint8List frame) {
  if (frame.isEmpty || frame.length > maxJoinRequestBytes) return null;

  final Object? parsed;
  try {
    parsed = jsonDecode(utf8.decode(frame));
  } catch (_) {
    // Not text, or not JSON: a bundle, or a stranger. Neither is this.
    return null;
  }
  if (parsed is! Map || !parsed.containsKey('join')) return null;

  if (parsed['pacc'] != joinVersion) {
    throw const SyncProtocolException(
      'the other device asked to join in a way this release does not '
      'understand; both devices need to be on the same version to pair',
    );
  }

  final Object? device = parsed['join'];
  if (device is! String || device.isEmpty) {
    throw const SyncProtocolException(
      'a device asked to join without saying which device it is, so there '
      'would be no way to record what it was given',
    );
  }
  return JoinRequest(device: device);
}
