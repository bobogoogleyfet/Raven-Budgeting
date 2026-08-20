// Tests that call the real core through the bridge.
//
// These need a device (`-d linux`), because they load the compiled `.so`.
// Anything provable without it belongs in `test/` instead, which runs far
// faster.

import 'dart:io';

import 'package:app/src/rust/api/categories.dart';
import 'package:app/src/rust/api/db.dart';
import 'package:app/src/rust/frb_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  late Directory dataDir;

  setUpAll(() async {
    await RustLib.init();
    // A temporary directory, never the real one. `openDatabase` creates the
    // file if it is absent, so pointing a test at the default path would
    // migrate and seed the user's own database.
    dataDir = Directory.systemTemp.createTempSync('pacc_bridge_test');
  });

  tearDownAll(() {
    dataDir.deleteSync(recursive: true);
  });

  test('opens a database at the directory it is given', () async {
    final path = await openDatabase(appDataDir: dataDir.path);

    expect(path, '${dataDir.path}/pacc.db');
    expect(File(path).existsSync(), isTrue);
  });

  test('re-opening keeps the handle it already has', () async {
    // What a Flutter hot restart does: Dart's `main()` runs again against a
    // library that was never unloaded.
    final first = await openDatabase(appDataDir: dataDir.path);
    final second = await openDatabase(appDataDir: dataDir.path);

    expect(second, first);
  });

  test('lists the seeded categories', () async {
    await openDatabase(appDataDir: dataDir.path);

    final categories = await listCategories();

    // 24 seeded categories across six groups, System included — `list`
    // excludes archived rows, not hidden ones.
    expect(categories, hasLength(24));

    final groceries = categories.firstWhere((c) => c.name == 'Groceries');
    expect(groceries.groupName, 'Frequent');
    expect(groceries.isBudgeted, isTrue);
    expect(groceries.id, greaterThan(0));

    expect(
      categories.map((c) => c.groupName).toSet(),
      containsAll(<String>['Bills', 'Frequent', 'Savings', 'System']),
    );
  });
}
