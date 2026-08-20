// Tests that call the real core through the bridge.
//
// These need a device (`-d linux`), because they load the compiled `.so`.
// Anything provable without it belongs in `test/` instead, which runs far
// faster.

import 'dart:io';

import 'package:app/src/api.dart';
import 'package:app/src/money.dart';
import 'package:app/src/rust/api/db.dart';
import 'package:app/src/widgets/cents_field.dart';
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

  test('a budget set from the app shows up in the month it starts', () async {
    await openDatabase(appDataDir: dataDir.path);
    // Hobbies rather than Groceries: an earlier test in this file files a
    // grocery receipt in the same month, and "what is left" would then depend
    // on the order the tests happened to run in.
    final hobbies =
        (await listCategories()).firstWhere((c) => c.name == 'Hobbies');

    final first = await setBudget(
      categoryId: hobbies.id,
      amount: fromCore(20000),
      period: BudgetPeriod.monthly,
      month: '2026-08',
    );
    expect(first.restated, isFalse);
    expect(first.closedAfter, isNull);

    final rows = await budgetStatus('2026-08');
    final row = rows.firstWhere((r) => r.categoryName == 'Hobbies');
    expect(row.isBudgeted, isTrue);
    expect(row.perMonth?.value, 20000);
    expect(row.remaining?.value, 20000, reason: 'nothing spent on hobbies');

    // The figure the overview leads with is the core's sum of the same rows.
    final totals = await monthTotals('2026-08');
    final expected = rows
        .where((r) => r.isBudgeted)
        .fold<int>(0, (sum, r) => sum + (r.perMonth?.value ?? 0));
    expect(totals.budgeted.value, expected);
  });

  test('setting the same month again restates rather than stacking', () async {
    await openDatabase(appDataDir: dataDir.path);
    final gas = (await listCategories()).firstWhere((c) => c.name == 'Gas');

    await setBudget(
      categoryId: gas.id,
      amount: fromCore(18000),
      period: BudgetPeriod.monthly,
      month: '2026-09',
    );
    final again = await setBudget(
      categoryId: gas.id,
      amount: fromCore(19000),
      period: BudgetPeriod.monthly,
      month: '2026-09',
    );

    // The user is correcting a figure they just entered, and rewriting it is
    // what they asked for.
    expect(again.restated, isTrue);
    final rows = await budgetStatus('2026-09');
    expect(
      rows.firstWhere((r) => r.categoryName == 'Gas').perMonth?.value,
      19000,
    );
  });

  test('a yearly budget contributes a twelfth of the bill each month',
      () async {
    await openDatabase(appDataDir: dataDir.path);
    final insurance =
        (await listCategories()).firstWhere((c) => c.name == 'Car Insurance');

    await setBudget(
      categoryId: insurance.id,
      amount: fromCore(78000),
      period: BudgetPeriod.yearly,
      month: '2026-01',
    );

    final row = (await budgetStatus('2026-08'))
        .firstWhere((r) => r.categoryName == 'Car Insurance');

    // $780 a year is $65 a month, and the whole bill stays the cycle's budget.
    expect(row.perMonth?.value, 6500);
    expect(row.budgeted?.value, 78000);
    expect(row.isMultiMonth, isTrue);
  });

  test('several amounts typed into one field come to their sum', () async {
    await openDatabase(appDataDir: dataDir.path);

    // Three items rung up under one category. The adding is the core's: a
    // frontend that does it has to do it again for every platform.
    expect(amountValue('1.38 + 1.84')?.value, 322);
    expect(amountValue('1.38 + 1.84 + 2.99')?.value, 621);

    // A part not typed yet is nothing, not a zero.
    expect(amountValue('1.38 + ')?.value, 138);
    expect(amountValue(''), isNull);

    // A refund still works, and still subtracts.
    expect(amountValue('-5.00 + 2.00')?.value, -300);
  });
}
