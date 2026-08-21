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
library;

import 'dart:convert';
import 'dart:typed_data';

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
