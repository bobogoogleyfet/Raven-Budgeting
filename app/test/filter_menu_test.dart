import 'package:app/src/widgets/filter_menu.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('nothing hidden is no filter at all, not a list of everything', () {
    // The two return the same rows today and different rows tomorrow: an
    // expense at a shop that did not exist when the menu was built would fail
    // the explicit list.
    expect(narrowedTo(['1', '2', '3'], {}), isNull);
  });

  test('hiding one asks for the rest, in the order the menu had them', () {
    // The menu is built alphabetically, and the query should not reshuffle it.
    expect(narrowedTo(['1', '2', '3'], {'2'}), ['1', '3']);
  });

  test('hiding everything asks for nothing, which is not the same as everything',
      () {
    // An empty list means "no rows match", and the caller must send it as
    // such. Collapsing it back to null would make Hide all show the whole
    // table.
    expect(narrowedTo(['1', '2'], {'1', '2'}), isEmpty);
    expect(narrowedTo(['1', '2'], {'1', '2'}), isNotNull);
  });

  test('a value that no longer exists stops hiding anything', () {
    // A store that was removed cannot keep filtering the list from beyond the
    // grave. Pruning it leaves nothing hidden, which is "no filter" again —
    // not an explicit list that quietly excludes whatever appears next.
    const options = [
      FilterOption(value: '1', label: 'Costco'),
      FilterOption(value: '3', label: 'Harmons'),
    ];
    final hidden = pruned({'2'}, options);

    expect(hidden, isEmpty);
    expect(narrowedTo(['1', '3'], hidden), isNull);
  });

  test('pruning keeps the values that are still there', () {
    const options = [
      FilterOption(value: '1', label: 'Costco'),
      FilterOption(value: '2', label: 'Chevron'),
    ];

    expect(pruned({'1', '9'}, options), {'1'});
  });
}
