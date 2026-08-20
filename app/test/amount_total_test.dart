@TestOn('vm')
library;

import 'package:app/src/widgets/cents_field.dart';
import 'package:flutter_test/flutter_test.dart';

// `amountValue` asks the core for the total, so it needs the real library —
// which is what the integration test covers. What can be checked here without
// it is the splitting, which is parsing rather than arithmetic.
void main() {
  test('splitting is by plus, and ignores what is not an amount yet', () {
    expect(amountParts('1.38 + 1.84').length, 2);
    expect(amountParts('1.38 + ').length, 1);
    expect(amountParts('+').length, 0);
    expect(amountParts('abc').length, 0);
  });
}
