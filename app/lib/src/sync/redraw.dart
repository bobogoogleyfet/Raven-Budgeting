import 'package:flutter/foundation.dart';

import 'lan_sync.dart';

/// Counts the exchanges that actually brought something in.
///
/// Screens read their data once, in `initState`. That is right for a screen
/// nothing else writes to, and wrong for one sitting in front of somebody
/// while a receipt lands from their other device — and worse than the local
/// case, because nobody touched this device. There is nothing to make them
/// suspect the list is stale, and no gesture they would naturally make to
/// refresh it.
///
/// Counting rather than signalling, so the value can go straight into a
/// widget key: a screen built under it is rebuilt exactly when this moves.
///
/// # Why not simply rebuild whenever the sync state changes
///
/// [LanSync] publishes on every beacon, on every peer appearing and dropping
/// out, and on every exchange that turned out to have nothing in it. Rebuilding
/// five screens for those would throw away scroll position and any half-typed
/// filter several times a minute, to show data that has not moved.
class SyncRedraws extends ValueNotifier<int> {
  SyncRedraws(this._source) : super(0) {
    _source.addListener(_consider);
  }

  final ValueListenable<LanSyncState> _source;

  /// The exchange last counted. Stamped once per exchange, so it separates two
  /// exchanges that each applied one change from a single exchange being
  /// republished — which happens, because a peer appearing rewrites the state
  /// without touching the result that came with it.
  DateTime? _counted;

  void _consider() {
    final state = _source.value;
    if ((state.lastApplied?.applied ?? 0) <= 0) return;
    final at = state.lastAt;
    if (at == null || at == _counted) return;
    _counted = at;
    value++;
  }

  @override
  void dispose() {
    _source.removeListener(_consider);
    super.dispose();
  }
}
