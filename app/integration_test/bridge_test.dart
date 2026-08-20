// Tests that call the real core through the bridge.
//
// These need a device (`-d linux`), because they load the compiled `.so`.
// Anything provable without it belongs in `test/` instead, which runs far
// faster.

import 'dart:io';

import 'package:app/src/api.dart';
import 'package:app/src/money.dart';
import 'package:app/src/rust/api/db.dart';
import 'package:app/src/rust/api/transactions.dart' as rust_txn;
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

  test('a month with no budget lines still lists its categories', () async {
    await openDatabase(appDataDir: dataDir.path);

    // Every budgeted category comes back even with no line in force: the
    // screen's most useful job is offering to budget something that is not
    // budgeted yet, and a row that is missing cannot make that offer.
    final rows = await budgetStatus('2026-08');

    expect(rows, isNotEmpty);
    expect(rows.every((r) => !r.isBudgeted), isTrue);
    expect(rows.every((r) => !r.isOver), isTrue);
    expect(rows.every((r) => r.funding == 'none'), isTrue);
    // System categories are not budgetable and must not appear.
    expect(rows.map((r) => r.groupName), isNot(contains('System')));
  });

  test('a malformed month is refused, not guessed at', () async {
    await openDatabase(appDataDir: dataDir.path);

    expect(() => budgetStatus('August'), throwsA(anything));
    expect(() => budgetStatus('2026-13'), throwsA(anything));
  });

  test('an empty database has no expenses and no filter values', () async {
    await openDatabase(appDataDir: dataDir.path);

    expect(await listTransactions(const TxnQuery(limit: 200)), isEmpty);

    final values = await filterValues();
    expect(values.merchants, isEmpty);
    expect(values.categories, isEmpty);
    expect(values.anyWithoutMerchant, isFalse);
  });

  test('an unknown sort order is refused rather than silently defaulted',
      () async {
    await openDatabase(appDataDir: dataDir.path);

    // A typo that quietly reorders a list is a bug nobody reports, because the
    // list still looks plausible.
    await expectLater(
      rust_txn.listTransactions(limit: 10, sort: 'dateDescending'),
      throwsA(anything),
    );
  });

  test('deleting an expense that is not there says so', () async {
    await openDatabase(appDataDir: dataDir.path);

    expect(() => deleteTransaction(9999), throwsA(anything));
  });

  test('a grocery subtotal is taxed at the reduced rate, end to end', () async {
    await openDatabase(appDataDir: dataDir.path);

    // The location has to be real before a draft can name it: the core refuses
    // one it does not know rather than falling back to no location, which
    // would switch the rate to 0% and produce a plausible, wrong number.
    final provo = await ensureLocation('Provo');
    expect(provo.taxRatePpm, greaterThan(0), reason: 'published rate');

    final groceries =
        (await listCategories()).firstWhere((c) => c.name == 'Groceries');

    final draft = SubtotalDraft(
      txnDate: '2026-08-20',
      merchant: "Macey's",
      location: 'Provo',
      rows: [DraftRow(categoryId: groceries.id, subtotal: fromCore(728))],
    );

    final preview = await previewSubtotals(draft);

    // $7.28 of groceries owes $0.22 and not $0.53: Utah's reduced rate on
    // unprepared food, worked out by the core and never by the frontend.
    expect(preview.subtotal.value, 728);
    expect(preview.tax.value, 22);
    expect(preview.total.value, 750);
    expect(preview.appliedRatePpm, 30000);
    // Nobody read this figure off a receipt, so it is flagged as an estimate.
    expect(preview.taxIsEstimated, isTrue);
    expect(preview.blockers, isEmpty);

    final id = await saveSubtotals(draft);
    expect(id, greaterThan(0));

    // What was previewed is what was written.
    final saved = (await listTransactions(const TxnQuery(limit: 10)))
        .firstWhere((t) => t.id == id);
    expect(saved.total.value, 750);
    expect(saved.merchantName, "Macey's");
    expect(saved.categories, ['Groceries']);
    expect(saved.taxIsEstimated, isTrue);
  });

  test('a corrected tax is the users, and stops being an estimate', () async {
    await openDatabase(appDataDir: dataDir.path);
    await ensureLocation('Provo');
    final groceries =
        (await listCategories()).firstWhere((c) => c.name == 'Groceries');

    // A receipt is evidence and a published rate is an estimate, so a figure
    // read off the paper wins.
    final preview = await previewSubtotals(SubtotalDraft(
      txnDate: '2026-08-20',
      location: 'Provo',
      rows: [
        DraftRow(
          categoryId: groceries.id,
          subtotal: fromCore(728),
          tax: fromCore(25),
        ),
      ],
    ));

    expect(preview.tax.value, 25);
    expect(preview.total.value, 753);
    expect(preview.taxIsEstimated, isFalse);
  });

  test('a row with no category blocks the save rather than guessing', () async {
    await openDatabase(appDataDir: dataDir.path);
    await ensureLocation('Provo');

    final preview = await previewSubtotals(SubtotalDraft(
      txnDate: '2026-08-20',
      location: 'Provo',
      rows: [DraftRow(categoryId: null, subtotal: fromCore(500))],
    ));

    expect(preview.blockers, isNotEmpty);
    expect(preview.canSave, isFalse);
    expect(() => saveSubtotals(SubtotalDraft(
          txnDate: '2026-08-20',
          location: 'Provo',
          rows: [DraftRow(categoryId: null, subtotal: fromCore(500))],
        )), throwsA(anything));
  });

  test('naming a location that does not exist is refused, not guessed at',
      () async {
    await openDatabase(appDataDir: dataDir.path);
    final groceries =
        (await listCategories()).firstWhere((c) => c.name == 'Groceries');

    // Falling back to "no location" would switch the rate to 0% and record a
    // plausible, wrong number — the worst kind of bug this app can have.
    expect(
      () => previewSubtotals(SubtotalDraft(
        txnDate: '2026-08-20',
        location: 'Nowhere At All',
        rows: [DraftRow(categoryId: groceries.id, subtotal: fromCore(728))],
      )),
      throwsA(anything),
    );
  });
}
