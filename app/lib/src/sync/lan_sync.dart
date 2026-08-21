/// Keeping two devices on one Wi-Fi up to date without anybody doing anything.
///
/// # What this is, and what it deliberately is not
///
/// The core already knows how to seal what a peer has not seen into a bundle
/// and how to apply one it is handed; it is forbidden from doing any
/// networking at all, for the reasons in `accounting-core`'s rule 1. So this
/// moves bytes and nothing else: no crypto of its own, no merge policy, no
/// idea what is inside a bundle. Everything on the wire is already encrypted
/// under the pairing phrase before this file sees it.
///
/// # Why pairing is still a button
///
/// Two devices that have each been opened have each seeded the same chart of
/// accounts under uuids of their own, and merging those gives two of
/// everything — one Groceries the receipts point at and one nobody uses. The
/// core's answer is [joinBundle], which adopts the other device's copy
/// wholesale, and which is destructive by design. So an unpaired device
/// exchanges nothing automatically: it broadcasts, it listens, it will hand
/// over its own data to a device that asks, and it offers *Join* to a person.
/// Once the two are paired their uuids agree, and from then on everything is
/// automatic.
///
/// Joining is what pairs *both* devices, and it has to: the one that adopts
/// records the device it took its data from, and the one that hands the data
/// over records the device it gave it to. Half of that is not a weaker
/// pairing, it is a pairing that works in one direction and silently refuses
/// everything sent back the other way.
///
/// # Who may connect
///
/// Anyone on the network. The pairing phrase is the group's identity and the
/// only credential there is: a stranger who connects gets a sealed bundle they
/// cannot open, and anything they send fails to open here and closes their
/// socket alone.
library;

import 'dart:async';

import 'package:flutter/foundation.dart';

import '../api.dart';
import 'discovery.dart';
import 'frames.dart';
import 'links.dart';

typedef LanSettingsLoader = Future<LanSettings> Function();
typedef LanSettingsSaver = Future<void> Function({
  required bool enabled,
  required String phrase,
});
typedef LanStatusLoader = Future<SyncStatus> Function();
typedef LanExporter = Future<Uint8List> Function({
  String? peer,
  required String phrase,
});
typedef LanMerger = Future<Applied> Function({
  required Uint8List bytes,
  required String phrase,
});
typedef LanMarker = Future<void> Function({
  String? peer,
  required String phrase,
  required Uint8List bytes,
});
typedef LanRefusalLoader = Future<String?> Function();

/// A device heard from recently.
@immutable
class LanPeer {
  const LanPeer({
    required this.device,
    required this.host,
    required this.port,
    required this.seenAt,
  });

  final String device;
  final String host;
  final int port;
  final DateTime seenAt;
}

/// Everything a screen needs to say about local-network sync.
///
/// Notably not the phrase. This object is held by a widget, rebuilt on every
/// change and printed by every debugging tool that walks a widget tree, which
/// is the last place a secret should be.
@immutable
class LanSyncState {
  const LanSyncState({
    this.enabled = false,
    this.phraseIsSet = false,
    this.listening = false,
    this.paired = false,
    this.peers = const <LanPeer>[],
    this.joinRefusal,
    this.problem,
    this.lastResult,
    this.lastFailed = false,
    this.lastApplied,
    this.lastAt,
  });

  /// The switch's position, which is not the same as [listening] — a device
  /// that was told to sync and could not bind a socket is on and not running,
  /// and saying so is the only way that gets noticed.
  final bool enabled;

  /// Whether a phrase has been stored. Never the phrase itself, and never
  /// repeated back to the screen it was typed on.
  final bool phraseIsSet;

  final bool listening;

  /// Whether this device has a peer at all. Until it has, nothing is exchanged
  /// automatically — see the note on joining in this library's docs.
  final bool paired;

  /// Devices heard from in the last few seconds, most recently seen first.
  final List<LanPeer> peers;

  /// Why this device could not adopt another's data, or null if it could.
  final String? joinRefusal;

  /// Something is stopping the service running at all: a port that would not
  /// bind, a permission Android did not grant.
  final String? problem;

  /// What the last exchange did, in a sentence.
  final String? lastResult;
  final bool lastFailed;

  /// The counts behind [lastResult], kept whole so the screen can show what
  /// did *not* arrive rather than only what did.
  final Applied? lastApplied;

  final DateTime? lastAt;

  LanSyncState copyWith({
    bool? enabled,
    bool? phraseIsSet,
    bool? listening,
    bool? paired,
    List<LanPeer>? peers,
    String? joinRefusal,
    bool clearJoinRefusal = false,
    String? problem,
    bool clearProblem = false,
    String? lastResult,
    bool? lastFailed,
    Applied? lastApplied,
    bool clearApplied = false,
    DateTime? lastAt,
  }) {
    return LanSyncState(
      enabled: enabled ?? this.enabled,
      phraseIsSet: phraseIsSet ?? this.phraseIsSet,
      listening: listening ?? this.listening,
      paired: paired ?? this.paired,
      peers: peers ?? this.peers,
      joinRefusal: clearJoinRefusal ? null : (joinRefusal ?? this.joinRefusal),
      problem: clearProblem ? null : (problem ?? this.problem),
      lastResult: lastResult ?? this.lastResult,
      lastFailed: lastFailed ?? this.lastFailed,
      lastApplied: clearApplied ? null : (lastApplied ?? this.lastApplied),
      lastAt: lastAt ?? this.lastAt,
    );
  }
}

/// The exchange service: discovery, transfer, and when to do either.
class LanSync {
  LanSync({
    this.loadSettings = lanSettings,
    this.saveSettings = saveLanSettings,
    this.loadStatus = syncStatus,
    this.loadRefusal = joinRefusal,
    this.export = exportBundle,
    this.merge = importBundle,
    this.adopt = joinBundle,
    this.markSent = markDelivered,
    this.openListener = openSocketListener,
    this.openLink = openSocketLink,
    this.openAnnouncer = openBroadcastSocket,
    this.port = discoveryPort,
    this.announceEvery = const Duration(seconds: 3),
    this.settleFor = const Duration(seconds: 1),
    this.catchUpEvery = const Duration(seconds: 60),
    this.forgetPeerAfter = const Duration(seconds: 12),
    this.patience = const Duration(seconds: 20),
  });

  /// Every call out of this object is injectable and defaults to the real
  /// thing, for the same reason the file-based sync section's are: a test has
  /// no Rust behind it and no business opening a socket on the machine
  /// running it.
  final LanSettingsLoader loadSettings;
  final LanSettingsSaver saveSettings;
  final LanStatusLoader loadStatus;
  final LanRefusalLoader loadRefusal;
  final LanExporter export;
  final LanMerger merge;
  final LanMerger adopt;
  final LanMarker markSent;
  final ListenerOpener openListener;
  final LinkOpener openLink;
  final AnnouncerOpener openAnnouncer;
  final int port;

  /// Often enough that a device that has just woken is found in a second or
  /// two, small enough that the datagram costs nothing.
  final Duration announceEvery;

  /// How long a burst of local edits is allowed to settle into one exchange.
  /// "Save & New" three times running is one thing that happened, not three.
  final Duration settleFor;

  /// The backstop. Every other trigger is an event that can be missed — a
  /// dropped datagram, a screen locked mid-write — and without this a device
  /// could sit a foot away from its peer, out of date, indefinitely.
  final Duration catchUpEvery;

  /// Four missed beacons. Long enough to ride out a lost datagram, short
  /// enough that a laptop that has been shut does not linger on screen as
  /// though it were there.
  final Duration forgetPeerAfter;

  /// How long any one socket operation may take before the exchange is
  /// abandoned. A stalled peer must cost one connection, not the service.
  final Duration patience;

  final ValueNotifier<LanSyncState> _state =
      ValueNotifier<LanSyncState>(const LanSyncState());

  /// What to show, and what to rebuild on.
  ValueListenable<LanSyncState> get state => _state;

  LanSettings _settings = const LanSettings(enabled: false, phrase: '');
  String _device = '';
  List<String> _knownPeers = const <String>[];

  final Map<String, LanPeer> _peers = <String, LanPeer>{};

  SyncListener? _listener;
  Announcer? _announcer;
  StreamSubscription<SyncLink>? _connections;
  StreamSubscription<Beam>? _beams;
  Timer? _announceTimer;
  Timer? _catchUpTimer;
  Timer? _settleTimer;

  /// Exchanges run one at a time, in the order they were asked for.
  ///
  /// Not a nicety: every one of them calls into the core, which holds a single
  /// mutex over the database, and two overlapping exchanges would interleave
  /// an export with somebody else's import — building a bundle that claims to
  /// cover changes the watermark had not caught up with. A queue costs
  /// milliseconds and removes the whole class of problem.
  Future<void> _turn = Future<void>.value();

  bool _disposed = false;

  /// Read what this device has been told, and start if it has been told to.
  ///
  /// Called once at launch. A failure here is not fatal to the app: sync being
  /// off is a normal state, and every screen works without it.
  Future<void> load() async {
    try {
      _settings = await loadSettings();
      _publish(
        (s) => s.copyWith(
          enabled: _settings.enabled,
          phraseIsSet: _settings.phrase.isNotEmpty,
        ),
      );
      await _refreshStanding();
      if (_settings.isRunnable) await _start();
    } catch (error) {
      _publish((s) => s.copyWith(problem: '$error'));
    }
  }

  /// Turn local-network sync on, with the phrase both devices share.
  ///
  /// An empty [phrase] means "the one already stored", which is what flicking
  /// the switch back on is: the phrase field is deliberately never filled in
  /// with what is saved, so the only way to turn sync on again would otherwise
  /// be to type the phrase from memory a second time.
  Future<void> turnOn([String phrase = '']) async {
    final chosen = phrase.isNotEmpty ? phrase : _settings.phrase;
    if (chosen.isEmpty) {
      _publish(
        (s) => s.copyWith(
          problem: 'Enter the pairing phrase first. It is the same phrase on '
              'both devices, and it is the only thing protecting what crosses '
              'the network.',
        ),
      );
      return;
    }
    await _apply(enabled: true, phrase: chosen);
  }

  /// Turn it off, keeping the phrase so turning it back on is one tap.
  ///
  /// Stopping is immediate and complete: the sockets close, so this device
  /// stops announcing itself and stops answering. A switch that left a
  /// listener running would be lying.
  Future<void> turnOff() => _apply(enabled: false, phrase: _settings.phrase);

  /// Replace the phrase without touching the switch.
  ///
  /// Refuses an empty one rather than clearing what is stored: an empty field
  /// is somebody who typed nothing, and reading that as "forget the phrase"
  /// would turn a stray tap into a silently dead sync.
  Future<void> changePhrase(String phrase) async {
    if (phrase.isEmpty) return;
    await _apply(enabled: _settings.enabled, phrase: phrase);
  }

  /// Something was written here; tell the peers, once the writing stops.
  ///
  /// Deliberately not a `Future`: a save must not wait on a network exchange,
  /// and nothing on screen is waiting for this.
  void nudge() {
    if (!_settings.isRunnable || _disposed) return;
    _settleTimer?.cancel();
    _settleTimer = Timer(settleFor, () {
      unawaited(_exchangeWithAll());
    });
  }

  /// Adopt a discovered device's data, replacing everything here.
  ///
  /// The one destructive thing in the feature, and the only one that needs a
  /// person. It pulls rather than exchanges: this device sends no data of its
  /// own — its chart of accounts is about to be thrown away, and handing it
  /// over first would only give the other device a duplicate of everything to
  /// merge. What it does send is its name, because the device about to hand
  /// over its whole ledger has to be able to record who it gave it to; that is
  /// the moment the two become peers, and it is the only moment either of them
  /// gets.
  Future<void> joinPeer(LanPeer peer) async {
    final refusal = await loadRefusal();
    if (refusal != null) {
      _publish((s) => s.copyWith(joinRefusal: refusal));
      _report(refusal, failed: true);
      return;
    }
    await _queue(() => _pullFrom(peer));
  }

  /// Stop everything and release the sockets.
  Future<void> dispose() async {
    _disposed = true;
    await _stop();
    _state.dispose();
  }

  // ---------------------------------------------------------------- settings

  Future<void> _apply({required bool enabled, required String phrase}) async {
    try {
      await saveSettings(enabled: enabled, phrase: phrase);
      _settings = LanSettings(enabled: enabled, phrase: phrase);
      _publish(
        (s) => s.copyWith(
          enabled: enabled,
          phraseIsSet: phrase.isNotEmpty,
          clearProblem: true,
        ),
      );
      await _stop();
      if (_settings.isRunnable) await _start();
    } catch (error) {
      // Never the phrase in a message, here or anywhere: errors are the thing
      // people paste into bug reports.
      _publish((s) => s.copyWith(problem: '$error'));
    }
  }

  // ------------------------------------------------------------ the sockets

  Future<void> _start() async {
    try {
      _device = (await loadStatus()).thisDevice;
      final listener = await openListener();
      _listener = listener;
      _connections = listener.connections.listen(
        (link) => unawaited(_queue(() => _serve(link))),
        // A listener that dies takes the feature with it silently, which is
        // the one failure mode worth interrupting somebody over.
        onError: (Object error) =>
            _publish((s) => s.copyWith(problem: 'Stopped listening: $error')),
      );

      final announcer = await openAnnouncer(port);
      _announcer = announcer;
      _beams = announcer.beams.listen(
        _heard,
        onError: (Object error) => _publish(
          (s) => s.copyWith(problem: 'Stopped hearing other devices: $error'),
        ),
      );

      _announce();
      _announceTimer = Timer.periodic(announceEvery, (_) => _announce());
      _catchUpTimer =
          Timer.periodic(catchUpEvery, (_) => unawaited(_exchangeWithAll()));
      _publish((s) => s.copyWith(listening: true, clearProblem: true));
      // Whatever happened while this device was off is waiting on the peers it
      // already knows, and none of them will be heard from for a few seconds.
      unawaited(_exchangeWithAll());
    } catch (error) {
      await _stop();
      _publish(
        (s) => s.copyWith(
          listening: false,
          problem: 'Could not start syncing over the network: $error',
        ),
      );
    }
  }

  Future<void> _stop() async {
    _announceTimer?.cancel();
    _catchUpTimer?.cancel();
    _settleTimer?.cancel();
    _announceTimer = null;
    _catchUpTimer = null;
    _settleTimer = null;
    // Cancelled, not awaited. The future a cancel hands back reports that the
    // subscription finished its own cleanup, which nothing here waits on — and
    // under a widget test's fake clock it never completes at all, because
    // nothing is turning the real event loop.
    unawaited(_beams?.cancel() ?? Future<void>.value());
    unawaited(_connections?.cancel() ?? Future<void>.value());
    _beams = null;
    _connections = null;
    _announcer?.close();
    _announcer = null;
    await _listener?.close();
    _listener = null;
    _peers.clear();
    if (!_disposed) {
      _publish((s) => s.copyWith(listening: false, peers: const <LanPeer>[]));
    }
  }

  void _announce() {
    final listener = _listener;
    final announcer = _announcer;
    if (listener == null || announcer == null || _device.isEmpty) return;
    try {
      announcer.announce(beaconBytes(device: _device, port: listener.port));
    } catch (error) {
      // A subnet that will not take a broadcast — a VPN, a locked-down guest
      // network — is a reason this device is not found, not a reason to stop
      // answering the ones that find it.
      _publish((s) => s.copyWith(problem: 'Could not announce this device: $error'));
    }
  }

  /// A datagram arrived. Most of them are this device's own.
  void _heard(Beam beam) {
    final beacon = readBeacon(beam.data, ownDevice: _device, from: beam.from);
    if (beacon == null) return;

    final known = _peers[beacon.device];
    final peer = LanPeer(
      device: beacon.device,
      host: beacon.host,
      port: beacon.port,
      seenAt: DateTime.now(),
    );
    _peers[beacon.device] = peer;
    _prunePeers();
    _publishPeers();

    // A device that has just appeared — or that has restarted onto a new port
    // — is the whole reason discovery exists. Anything else is covered by the
    // catch-up, and exchanging on every beacon would be an exchange every
    // three seconds forever.
    final isNew = known == null || known.port != peer.port || known.host != peer.host;
    if (isNew) unawaited(_queue(() => _exchange(peer)));
  }

  void _prunePeers() {
    final cutoff = DateTime.now().subtract(forgetPeerAfter);
    _peers.removeWhere((_, peer) => peer.seenAt.isBefore(cutoff));
  }

  // ---------------------------------------------------------- the exchanges

  Future<void> _exchangeWithAll() async {
    if (!_settings.isRunnable) return;
    _prunePeers();
    for (final peer in _peers.values.toList()) {
      await _queue(() => _exchange(peer));
    }
  }

  /// One exchange, as the device that dialled.
  ///
  /// Only with a device this one already syncs with. A bundle from a device
  /// that has never been paired with carries its own seeded chart of accounts,
  /// and merging that produces a second copy of every category — which is what
  /// joining exists to prevent and cannot be undone afterwards.
  Future<void> _exchange(LanPeer peer) async {
    if (!_settings.isRunnable || _disposed) return;
    if (!_knownPeers.contains(peer.device)) return;

    final phrase = _settings.phrase;
    SyncLink? link;
    FrameReader? reader;
    try {
      link = await openLink(peer.host, peer.port).timeout(patience);
      reader = FrameReader(link.incoming);

      // Addressed to the device actually on the other end of this socket, not
      // to "the peer" — with three devices in a household, a bundle built for
      // one and marked delivered to another would convince this device it had
      // sent changes that nobody received.
      final ours = await export(peer: peer.device, phrase: phrase);
      link.send(framed(ours));
      await link.flush();

      final theirs = await reader.next().timeout(patience);
      Applied? applied;
      if (theirs.isNotEmpty) {
        applied = await merge(bytes: theirs, phrase: phrase);
      }

      // The half-close is the receipt this end gives: the far end waits for it
      // before recording anything as delivered, and it goes out only now that
      // what it sent has actually been applied here.
      await link.finishSending();

      // And the receipt this end accepts. The peer replies only after it has
      // imported what was sent, so a reply — even an empty one — is proof the
      // bundle landed, which is the only thing worth moving a watermark on.
      await markSent(peer: peer.device, phrase: phrase, bytes: ours);

      await _refreshStanding();
      _report(_sentence(peer.device, applied), applied: applied);
    } catch (error) {
      _report('Could not sync with ${peer.device}: $error', failed: true);
    } finally {
      await reader?.cancel();
      link?.abandon();
    }
  }

  /// One exchange, as the device that answered.
  ///
  /// Whoever this is, they are on the network and they either have the phrase
  /// or they do not. Nothing here is trusted before the core has opened what
  /// arrived, and every failure closes this socket and only this socket.
  Future<void> _serve(SyncLink link) async {
    if (!_settings.isRunnable || _disposed) return;

    final phrase = _settings.phrase;
    FrameReader? reader;
    try {
      reader = FrameReader(link.incoming);
      final theirs = await reader.next().timeout(patience);
      final joining = readJoinRequest(theirs);
      if (joining != null && joining.device == _device) {
        // This device's own name for its database, from something that is not
        // this device: a loopback, or a stranger echoing back a beacon. An
        // export addressed to a peer skips every change that peer authored, so
        // answering would hand over an empty history dressed as a whole one,
        // and then record it as delivered.
        throw SyncProtocolException(
          'a device asked to join under this device’s own name, which is not '
          'a name it can be given anything under',
        );
      }

      Applied? applied;
      if (joining != null) {
        // Nothing to merge, by design: a joiner is seconds away from throwing
        // its own copy away, so it sends its name in place of a bundle.
      } else if (theirs.isEmpty) {
        // An older release joining, which says nothing about itself. Answered
        // in full below, and unrecordable — see [_bundleFor].
      } else if (_knownPeers.isEmpty) {
        // Unpaired, so this device's uuids are its own and merging would
        // double its chart of accounts. Say so rather than silently dropping
        // it: from the other end this looks like sync not working.
        //
        // Still only *merging* is refused. Answering a join is safe however
        // unpaired this device is — it hands over data and takes none — and
        // refusing that as well is what made pairing unresolvable: becoming
        // paired needed an import, and importing needed to be paired already.
        _report(
          'Another device offered to sync, but this one is not paired with '
          'anything yet. Join it from the list below to adopt its data.',
          failed: true,
        );
      } else {
        applied = await merge(bytes: theirs, phrase: phrase);
      }

      final ours = await _bundleFor(
        joining: joining?.device,
        pull: theirs.isEmpty,
        phrase: phrase,
      );
      link.send(framed(ours));
      await link.flush();

      // Wait for the other end to hang up. It does that only once it has
      // applied what was just sent, so this is the strongest evidence a socket
      // can offer that the bundle arrived — and a watermark advanced without
      // it would mean these changes were never sent again.
      await reader.untilClosed().timeout(patience);

      if (joining != null) {
        // The pairing moment on this side, and the only one it gets: this
        // device has just handed over its entire history to a device that
        // named itself. `merge::mark_sent` writes the `sync_peer` row as well
        // as the watermark, so this is what makes the joiner a peer here —
        // without it this device would go on refusing everything the joiner
        // sent while both screens claimed the two were paired.
        //
        // The evidence is the half-close waited for above, and it is exactly
        // the evidence the ordinary reply below already advances a watermark
        // on: the joiner adopts what it was handed and closes afterwards. What
        // it is not is proof the adopt committed — a device that died between
        // receiving and applying closes its socket the same way. So rather
        // than pretend a join has stronger evidence than it does, the join is
        // made repeatable: see [_bundleFor], which hands the whole history
        // over again to a joiner this device has already marked one against.
        await markSent(peer: joining.device, phrase: phrase, bytes: ours);
        await _refreshStanding();
        _report(
          '${joining.device} joined this device: it now carries a copy of '
          'everything here, and the two sync from now on.',
        );
      } else if (applied != null) {
        if (ours.isNotEmpty) {
          await markSent(phrase: phrase, bytes: ours);
        }
        await _refreshStanding();
        _report(_sentence(link.peer, applied), applied: applied);
      }
    } catch (error) {
      // A wrong phrase lands here, as does a truncated frame and a port
      // scanner. None of them is allowed to be more than one closed socket:
      // the listener is still up when this returns.
      _report('An exchange failed: $error', failed: true);
    } finally {
      await reader?.cancel();
      link.abandon();
    }
  }

  /// Adopt what a peer hands over, replacing everything here.
  Future<void> _pullFrom(LanPeer peer) async {
    final phrase = _settings.phrase;
    SyncLink? link;
    FrameReader? reader;
    try {
      link = await openLink(peer.host, peer.port).timeout(patience);
      reader = FrameReader(link.incoming);

      // This device's name, and nothing else. Sending its own bundle instead
      // would hand the other device a duplicate of every category, seconds
      // before this one throws that copy away — but sending *nothing* is no
      // better, because the far end is about to give away its whole ledger and
      // has to know who it gave it to. Both the reply and the peer row the
      // other device records are addressed to this name.
      link.send(framed(joinRequestBytes(device: _device)));
      await link.flush();

      final theirs = await reader.next().timeout(patience);
      if (theirs.isEmpty) {
        _report(
          '${peer.device} had nothing to hand over — it has no data of its '
          'own yet. Start from the device your expenses are on.',
          failed: true,
        );
        return;
      }
      final applied = await adopt(bytes: theirs, phrase: phrase);
      await link.finishSending();
      await _refreshStanding();
      _report(
        'Joined ${peer.device}: this device now carries its data. '
        '${_counts(applied)}',
        applied: applied,
      );
    } catch (error) {
      _report('Could not join ${peer.device}: $error', failed: true);
    } finally {
      await reader?.cancel();
      link?.abandon();
    }
  }

  /// What to reply with, addressed to whoever this can honestly address.
  ///
  /// A *join* is answered with everything, and addressed to [joining] — a
  /// device this one has never met, so the core finds no watermark under it
  /// and seals the whole history. Naming the joiner is what lets the handover
  /// be recorded against a device that exists, which is the whole of the fix.
  ///
  /// Unless there *is* a watermark under that name, which means this is a
  /// second attempt: the first was written off as delivered on the strength of
  /// a half-close, and whatever the joiner did with it plainly did not last.
  /// Addressing the retry to the joiner would answer it with the tail of a
  /// history it never received, and a join wipes the device before applying
  /// what it is given. So a retry falls back to the unwatermarked name and
  /// hands the whole thing over again. Marking it stays keyed to the real
  /// device either way — the core reads the watermark out of the bundle's
  /// contents, not out of who it was addressed to.
  ///
  /// An older release joins with an empty frame instead and says nothing about
  /// itself. It still gets everything — addressed to [_joining], which no
  /// device is called, so again there is no watermark to find — but this end
  /// cannot pair from it, having no name to write down. That pairing stays
  /// one-directional until the other device is updated, which is the most this
  /// end can do with what it was told.
  ///
  /// An ordinary exchange is addressed to the only peer this device has, and
  /// with several it cannot be addressed at all — a socket carries no device
  /// name, so there is no way to tell which of them dialled. That case replies
  /// empty and lets this device's own outgoing exchange, which does know who
  /// it is talking to, carry the changes a moment later.
  Future<Uint8List> _bundleFor({
    required String? joining,
    required bool pull,
    required String phrase,
  }) async {
    if (joining != null) {
      final retry = _knownPeers.contains(joining);
      return export(peer: retry ? _joining : joining, phrase: phrase);
    }
    if (pull) return export(peer: _joining, phrase: phrase);
    if (_knownPeers.length > 1) return Uint8List(0);
    return export(peer: null, phrase: phrase);
  }

  /// The addressee of a bundle nobody is keeping a watermark for.
  ///
  /// An older release's join, and a retried one. Parenthesised so it cannot
  /// collide with a real device name, which is always a uuid — the same trick,
  /// for the same reason, as the core's own `(unpaired)`.
  static const String _joining = '(joining)';

  // ------------------------------------------------------------ bookkeeping

  /// Ask the core who this device is and who it syncs with.
  ///
  /// Re-read after every exchange rather than cached: an import is exactly the
  /// moment a device gains its first peer, and the answer decides whether the
  /// next connection is allowed to merge anything at all.
  Future<void> _refreshStanding() async {
    final status = await loadStatus();
    _device = status.thisDevice;
    _knownPeers = status.peers;
    final refusal = status.isPaired ? null : await loadRefusal();
    _publish(
      (s) => s.copyWith(
        paired: status.isPaired,
        joinRefusal: refusal,
        clearJoinRefusal: refusal == null,
      ),
    );
  }

  /// Run [work] after everything already asked for, and never let a failure
  /// break the chain for whatever is queued behind it.
  Future<void> _queue(Future<void> Function() work) {
    _turn = _turn.then((_) => work()).catchError((Object _) {});
    return _turn;
  }

  String _sentence(String peer, Applied? applied) {
    if (applied == null) return 'Sent this device’s changes to $peer.';
    return 'Synced with $peer. ${_counts(applied)}';
  }

  String _counts(Applied applied) =>
      '${applied.applied} applied, ${applied.ignored} already here, '
      '${applied.conflicts} disagreed.';

  void _report(String what, {bool failed = false, Applied? applied}) {
    _publish(
      (s) => s.copyWith(
        lastResult: what,
        lastFailed: failed,
        lastApplied: applied,
        clearApplied: applied == null,
        lastAt: DateTime.now(),
      ),
    );
  }

  void _publishPeers() {
    final peers = _peers.values.toList()
      ..sort((a, b) => b.seenAt.compareTo(a.seenAt));
    _publish((s) => s.copyWith(peers: peers));
  }

  void _publish(LanSyncState Function(LanSyncState) change) {
    if (_disposed) return;
    _state.value = change(_state.value);
  }
}
