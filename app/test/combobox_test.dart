import 'package:app/src/widgets/combobox.dart';
import 'package:flutter_test/flutter_test.dart';

const List<String> stores = ["Macey's", 'Walmart', 'Maverik', 'Amazon'];

void main() {
  test('an empty query offers everything, in the order given', () {
    // Most-recently-shopped first, which is the order the core returns.
    expect(filterOptions(stores, ''), stores);
    expect(filterOptions(stores, '   '), stores);
  });

  test('prefix matches come before interior ones', () {
    // "ma" offers Macey's before Walmart: the store whose name starts that way
    // is nearly always the one meant.
    expect(filterOptions(stores, 'ma'), ["Macey's", 'Maverik', 'Walmart', 'Amazon']);
  });

  test('matching ignores case and surrounding space', () {
    expect(filterOptions(stores, '  WAL '), ['Walmart']);
  });

  test('nothing matching is an empty list, not everything', () {
    expect(filterOptions(stores, 'zzz'), isEmpty);
  });

  test('a prefix is not a known name', () {
    // "Cost" must still offer to be added, because "Cost Plus" is a different
    // shop from Costco.
    expect(isKnown(['Costco'], 'Cost'), isFalse);
    expect(isKnown(['Costco'], 'costco'), isTrue);
    expect(isKnown(['Costco'], '  Costco  '), isTrue);
    expect(isKnown(['Costco'], ''), isFalse);
  });
}
