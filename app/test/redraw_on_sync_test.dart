// Deciding when an arrival is worth redrawing for.
//
// Screens read their data once, in `initState`. That is right for a screen
// nothing else writes to, and wrong for one sitting in front of somebody while
// a receipt lands from their other device — and worse than the local case,
// because nobody touched this device, so there is nothing to make them suspect
// the list is stale and no gesture they would naturally make to refresh it.
import 'package:app/src/api.dart';
import 'package:app/src/sync/lan_sync.dart';
import 'package:app/src/sync/redraw.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';

Applied applied(int n) => Applied(
      applied: n,
      ignored: 0,
      conflicts: 0,
      deferred: const [],
      unknownKinds: const [],
    );

void main() {
  test('an exchange that applied something counts', () {
    final source = ValueNotifier(const LanSyncState());
    final redraws = SyncRedraws(source);
    addTearDown(() {
      redraws.dispose();
      source.dispose();
    });

    expect(redraws.value, 0);
    source.value = LanSyncState(
      lastApplied: applied(3),
      lastAt: DateTime(2026, 8, 21, 10),
    );
    expect(redraws.value, 1, reason: 'three changes arrived');
  });

  test('an exchange that brought nothing does not', () {
    // The service publishes on every beacon, every peer appearing and
    // dropping, and every empty exchange. Rebuilding for those would throw
    // away scroll position and any half-typed filter several times a minute,
    // to show data that has not moved.
    final source = ValueNotifier(const LanSyncState());
    final redraws = SyncRedraws(source);
    addTearDown(() {
      redraws.dispose();
      source.dispose();
    });

    source.value = LanSyncState(
      lastApplied: applied(0),
      lastAt: DateTime(2026, 8, 21, 10),
    );
    source.value = const LanSyncState(listening: true);
    expect(redraws.value, 0);
  });

  test('one exchange republished is one event, a later one is another', () {
    // A peer appearing rewrites the state without touching the result that
    // came with it, so the same arrival is published more than once.
    final source = ValueNotifier(const LanSyncState());
    final redraws = SyncRedraws(source);
    addTearDown(() {
      redraws.dispose();
      source.dispose();
    });

    final at = DateTime(2026, 8, 21, 10);
    source.value = LanSyncState(lastApplied: applied(1), lastAt: at);
    source.value =
        LanSyncState(lastApplied: applied(1), lastAt: at, listening: true);
    expect(redraws.value, 1, reason: 'republished, not re-arrived');

    source.value = LanSyncState(
      lastApplied: applied(1),
      lastAt: DateTime(2026, 8, 21, 10, 1),
    );
    expect(redraws.value, 2, reason: 'a second exchange is a second event');
  });

  test('it stops listening once disposed', () {
    // A listener left on a notifier that outlives the widget is the usual way
    // a rebuild fires into a dead tree.
    final source = ValueNotifier(const LanSyncState());
    final redraws = SyncRedraws(source)..dispose();

    source.value = LanSyncState(
      lastApplied: applied(5),
      lastAt: DateTime(2026, 8, 21, 11),
    );
    expect(redraws.value, 0);
    source.dispose();
  });
}
