/// The sockets, narrowed to the four things sync actually does with them.
///
/// `dart:io`'s `Socket`, `ServerSocket` and `RawDatagramSocket` are large
/// classes with dozens of members, and a unit test that had to stand in for one
/// would either open a real port — making the test depend on the machine's
/// network, its firewall, and on no other test binding the same port — or
/// implement a hundred members to reach the four that matter. So the service
/// talks to these three interfaces instead, and `dart:io` appears in exactly
/// one place: the implementations at the bottom of this file.
library;

import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

/// A byte pipe to one peer, for the length of one exchange.
abstract class SyncLink {
  /// Whatever the peer sends, in whatever chunks the network delivers it.
  Stream<Uint8List> get incoming;

  /// Somewhere to name this end's partner in a message. An address, not an
  /// identity: the device name is inside the sealed bundle, and nothing
  /// unauthenticated on a socket is worth calling a name.
  String get peer;

  /// Queue bytes for sending.
  void send(List<int> bytes);

  /// Wait until what was queued has reached the operating system.
  Future<void> flush();

  /// Say "nothing more from me" without stopping listening.
  ///
  /// A half-close, and the acknowledgement the protocol runs on: the far end
  /// sees it only once this end has read and applied what it was sent, so it
  /// is the one signal a watermark may safely be advanced on.
  Future<void> finishSending();

  /// Drop the connection now, whatever state it is in. For the failure path:
  /// a session that threw has nothing left to say and should not leave a
  /// socket open while it decides that.
  void abandon();
}

/// Somewhere for peers to connect to.
abstract class SyncListener {
  /// The port the operating system actually gave out, which is what gets
  /// broadcast. Chosen by the OS rather than fixed, so two copies on one
  /// machine — an app and a test, a phone tethered to a laptop — cannot fight
  /// over it.
  int get port;

  Stream<SyncLink> get connections;

  Future<void> close();
}

/// The broadcast socket every device shouts on and listens to.
abstract class Announcer {
  /// Every datagram that arrives, this device's own broadcasts included.
  Stream<Beam> get beams;

  /// Shout to the whole subnet.
  void announce(List<int> bytes);

  void close();
}

/// One datagram, and where it came from.
class Beam {
  const Beam({required this.data, required this.from});

  final Uint8List data;

  /// The sender's address. The port it came *from* is not the port to reply
  /// on — the beacon carries that, because the TCP listener is on a different
  /// one entirely.
  final String from;
}

/// How a service opens each of the three. Injected so a test can supply pipes.
typedef ListenerOpener = Future<SyncListener> Function();
typedef LinkOpener = Future<SyncLink> Function(String host, int port);
typedef AnnouncerOpener = Future<Announcer> Function(int port);

/// A [SyncLink] over a real TCP socket.
class SocketLink implements SyncLink {
  SocketLink(this._socket);

  final Socket _socket;

  @override
  Stream<Uint8List> get incoming => _socket;

  @override
  String get peer => '${_socket.remoteAddress.address}:${_socket.remotePort}';

  @override
  void send(List<int> bytes) => _socket.add(bytes);

  @override
  Future<void> flush() => _socket.flush();

  @override
  Future<void> finishSending() => _socket.close();

  @override
  void abandon() => _socket.destroy();
}

/// A [SyncListener] over a real `ServerSocket`.
class SocketListener implements SyncListener {
  SocketListener(this._server);

  final ServerSocket _server;

  @override
  int get port => _server.port;

  @override
  Stream<SyncLink> get connections => _server.map(SocketLink.new);

  @override
  Future<void> close() => _server.close();
}

/// An [Announcer] over a real UDP socket.
class BroadcastAnnouncer implements Announcer {
  BroadcastAnnouncer(this._socket, this._port);

  final RawDatagramSocket _socket;
  final int _port;

  /// `255.255.255.255` rather than the subnet's own broadcast address: working
  /// the latter out needs the interface's netmask, and a device with several
  /// interfaces — Wi-Fi, a VPN, a docking station — has several right answers.
  /// The limited broadcast address is the one every host on the link accepts
  /// and no router forwards, which is exactly the reach this feature wants.
  static final InternetAddress everyone = InternetAddress('255.255.255.255');

  @override
  Stream<Beam> get beams => _socket
      .where((event) => event == RawSocketEvent.read)
      .map((_) => _socket.receive())
      .where((datagram) => datagram != null)
      .map(
        (datagram) => Beam(
          data: datagram!.data,
          from: datagram.address.address,
        ),
      );

  @override
  void announce(List<int> bytes) => _socket.send(bytes, everyone, _port);

  @override
  void close() => _socket.close();
}

/// Listen for exchanges on a port the operating system picks.
Future<SyncListener> openSocketListener() async =>
    SocketListener(await ServerSocket.bind(InternetAddress.anyIPv4, 0));

/// Connect to a peer that announced itself.
Future<SyncLink> openSocketLink(String host, int port) async =>
    SocketLink(await Socket.connect(host, port));

/// Bind the shared broadcast port.
///
/// `reuseAddress` and `reusePort` because every device on the network binds the
/// *same* port, and on one machine that includes a second copy of this app —
/// a desktop build and a phone on the same laptop during development. Without
/// them the second one fails to start and the feature looks broken.
Future<Announcer> openBroadcastSocket(int port) async {
  final socket = await RawDatagramSocket.bind(
    InternetAddress.anyIPv4,
    port,
    reuseAddress: true,
    reusePort: true,
  );
  socket.broadcastEnabled = true;
  return BroadcastAnnouncer(socket, port);
}
