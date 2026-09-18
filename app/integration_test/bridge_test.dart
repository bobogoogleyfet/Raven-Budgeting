// Tests that call the real core through the bridge.
//
// These need a device (`-d linux`), because they load the compiled `.so`.
// Anything provable without it belongs in `test/` instead, which runs far
// faster.

import 'dart:io';
import 'dart:typed_data';

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
    final path = await openDatabase(appDataDir: dataDir.path, demo: false);

    expect(path, '${dataDir.path}/pacc.db');
    expect(File(path).existsSync(), isTrue);
  });

  test('re-opening keeps the handle it already has', () async {
    // What a Flutter hot restart does: Dart's `main()` runs again against a
    // library that was never unloaded.
    final first = await openDatabase(appDataDir: dataDir.path, demo: false);
    final second = await openDatabase(appDataDir: dataDir.path, demo: false);

    expect(second, first);
  });

  test('lists the seeded categories', () async {
    await openDatabase(appDataDir: dataDir.path, demo: false);

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
    await openDatabase(appDataDir: dataDir.path, demo: false);

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
    await openDatabase(appDataDir: dataDir.path, demo: false);

    expect(() => budgetStatus('August'), throwsA(anything));
    expect(() => budgetStatus('2026-13'), throwsA(anything));
  });

  test('an empty database has no expenses and no filter values', () async {
    await openDatabase(appDataDir: dataDir.path, demo: false);

    expect(await listTransactions(const TxnQuery(limit: 200)), isEmpty);

    final values = await filterValues();
    expect(values.merchants, isEmpty);
    expect(values.categories, isEmpty);
    expect(values.anyWithoutMerchant, isFalse);
  });

  test('an unknown sort order is refused rather than silently defaulted',
      () async {
    await openDatabase(appDataDir: dataDir.path, demo: false);

    // A typo that quietly reorders a list is a bug nobody reports, because the
    // list still looks plausible.
    await expectLater(
      rust_txn.listTransactions(limit: 10, sort: 'dateDescending'),
      throwsA(anything),
    );
  });

  test('a date range narrows the list and its totals together', () async {
    await openDatabase(appDataDir: dataDir.path, demo: false);
    await ensureLocation('Provo');
    final groceries =
        (await listCategories()).firstWhere((c) => c.name == 'Groceries');

    Future<int> save(String date, int cents) => saveSubtotals(SubtotalDraft(
          txnDate: date,
          merchant: 'Ranges',
          location: 'Provo',
          rows: [DraftRow(categoryId: groceries.id, subtotal: fromCore(cents))],
        ));

    // The days either side of the month, and the days at its edges: the
    // expense list is paged by month now, so an off-by-one at either bound
    // moves a receipt into a month it does not belong to and takes its money
    // with it.
    final int july = await save('2026-07-31', 100);
    final int first = await save('2026-08-01', 200);
    final int last = await save('2026-08-31', 300);
    final int september = await save('2026-09-01', 400);

    const august = TxnQuery(limit: 500, from: '2026-08-01', to: '2026-08-31');
    final rows = await listTransactions(august);
    final ids = rows.map((r) => r.id).toList();
    expect(ids, containsAll([first, last]), reason: 'both bounds inclusive');
    expect(ids, isNot(contains(july)));
    expect(ids, isNot(contains(september)));

    // The bottom line is summed by the core over the same bounds, so it counts
    // exactly what the list shows rather than the whole database.
    final totals = await transactionTotals(august);
    expect(totals.expenses, ids.length);

    // A bound that is not a date is refused rather than dropped: a range whose
    // lower bound quietly vanished would answer a wider question than the one
    // on screen, and look right doing it.
    await expectLater(
      rust_txn.listTransactions(limit: 10, from: 'August'),
      throwsA(anything),
    );
  });

  test('deleting an expense that is not there says so', () async {
    await openDatabase(appDataDir: dataDir.path, demo: false);

    expect(() => deleteTransaction(9999), throwsA(anything));
  });

  test('a grocery subtotal is taxed at the reduced rate, end to end', () async {
    await openDatabase(appDataDir: dataDir.path, demo: false);

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
    await openDatabase(appDataDir: dataDir.path, demo: false);
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
    await openDatabase(appDataDir: dataDir.path, demo: false);
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
    await openDatabase(appDataDir: dataDir.path, demo: false);
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
    await openDatabase(appDataDir: dataDir.path, demo: false);
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
    await openDatabase(appDataDir: dataDir.path, demo: false);
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
    await openDatabase(appDataDir: dataDir.path, demo: false);
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
    await openDatabase(appDataDir: dataDir.path, demo: false);

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

  test('this device names itself once and keeps that name', () async {
    await openDatabase(appDataDir: dataDir.path, demo: false);

    final first = await thisDevice();
    final second = await thisDevice();

    // The name is minted on demand and then belongs to the database: peers
    // track how far along it is by that name, so a second answer would look
    // to them like a third device.
    expect(first, isNotEmpty);
    expect(second, first);
    expect((await syncStatus()).thisDevice, first);
  });

  test('a sealed bundle only opens with the phrase it was sealed with',
      () async {
    await openDatabase(appDataDir: dataDir.path, demo: false);

    final bytes = await exportBundle(phrase: 'seven rusty kettles');

    // The magic is what makes a wrong file chosen in a picker fail with "this
    // is not a sync file" rather than with "the phrase is wrong", which would
    // send somebody hunting for a typo that was never there.
    expect(String.fromCharCodes(bytes.take(8)), 'PACCSYNC');

    Object? refused;
    try {
      await importBundle(bytes: bytes, phrase: 'seven rusty kettes');
    } catch (error) {
      refused = error;
    }
    expect(refused, isNotNull);
    // Never the phrase, in the message or anywhere near it.
    expect('$refused'.contains('kett'), isFalse);

    // The right phrase opens it: a wrong one would have thrown, as above.
    // What it holds is this device's own history, so reading it back takes
    // nothing and disagrees with nothing — it used to record every entry in
    // it as a conflict.
    final applied = await importBundle(bytes: bytes, phrase: 'seven rusty kettles');
    expect(applied.applied, 0);
    expect(applied.conflicts, 0);
    expect(applied.deferred, isEmpty);
    expect(applied.unknownKinds, isEmpty);
  });

  test('an empty phrase is refused rather than sealing an open file', () async {
    await openDatabase(appDataDir: dataDir.path, demo: false);

    // The phrase is the only thing protecting a whole ledger left in a synced
    // folder; an empty one derives a key anybody else can derive too.
    expect(
      () => exportBundle(phrase: ''),
      throwsA(isA<Object>()),
    );
  });

  test('the first bundle of a pairing records no peer', () async {
    await openDatabase(appDataDir: dataDir.path, demo: false);

    final bytes = await exportBundle(phrase: 'seven rusty kettles');
    await markDelivered(phrase: 'seven rusty kettles', bytes: bytes);

    // A watermark names a device, and there is none yet. Inventing one would
    // make it the "only peer" every later export addressed itself to while the
    // real device's mark stayed empty.
    //
    // Filtered by name rather than asserted empty because the test above
    // imported *this* database's own bundle, and `merge::import` records a
    // peer row for whoever the bundle came from — which in that one case is
    // this device. No file from a real second device can do that: it carries
    // a different name.
    final status = await syncStatus();
    expect(status.peers.where((peer) => peer != status.thisDevice), isEmpty);
  });

  test('a file for a new device is sealed, and is not one to join from here',
      () async {
    await openDatabase(appDataDir: dataDir.path, demo: false);
    final before = await listCategories();

    final bytes = await exportForNewDevice(phrase: 'seven rusty kettles');
    expect(String.fromCharCodes(bytes.take(8)), 'PACCSYNC');
    expect(() => exportForNewDevice(phrase: ''), throwsA(isA<Object>()));

    // Joining it back here would throw this device's records away for its
    // own copy of them. Refused — for that reason, or because this test
    // database has already been paired or given work by the tests above —
    // and either way before anything is replaced.
    Object? refused;
    try {
      await joinBundle(bytes: bytes, phrase: 'seven rusty kettles');
    } catch (error) {
      refused = error;
    }
    expect(refused, isNotNull);
    expect((await listCategories()).length, before.length);
  });

  test('a file that is not a rate workbook is refused and the rates stay',
      () async {
    await openDatabase(appDataDir: dataDir.path, demo: false);
    final before = await rateRelease();

    // Thrown, not reported as a refresh's `problem`: the user chose this
    // file, and a quiet "not installed" would read as if it had worked.
    await expectLater(
      installRateWorkbook(Uint8List.fromList('not a workbook'.codeUnits)),
      throwsA(isA<Object>()),
    );

    final after = await rateRelease();
    expect(after?.quarter, before?.quarter);
  });
}
