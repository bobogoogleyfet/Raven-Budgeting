import 'dart:async';

import 'package:app/src/money.dart';
import 'package:app/src/widgets/cents_field.dart';

/// Widget tests run without the compiled Rust library, so anything that would
/// call into it has to be stood in for. Only the addition is: everything else
/// a test touches is Dart already.
///
/// The real one is the core's, and `integration_test/` checks that the two
/// agree — including that `1.38 + 1.84` is 322 cents there as well as here.
Future<void> testExecutable(FutureOr<void> Function() testMain) async {
  addUp = (parts) => fromCore(
        parts.fold<int>(0, (sum, part) => sum + part.value),
      );
  await testMain();
}
