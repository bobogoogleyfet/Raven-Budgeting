import 'package:app/src/api.dart';
import 'package:app/src/money.dart';
import 'package:app/src/screens/expenses_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

TxnSummary expense(
  int id, {
  String? merchant = "Macey's",
  int total = 11840,
  String date = '2026-08-18',
  List<String> categories = const ['Groceries'],
  bool needsReview = false,
}) =>
    TxnSummary(
      id: id,
      txnDate: date,
      merchantName: merchant,
      total: fromCore(total),
      taxIsEstimated: false,
      needsReview: needsReview,
      categories: categories,
    );

const FilterValues someValues = FilterValues(
  merchants: [
    FilterValue(id: 1, name: 'Chevron'),
    FilterValue(id: 2, name: "Macey's"),
  ],
  categories: [
    FilterValue(id: 10, name: 'Gas'),
    FilterValue(id: 11, name: 'Groceries'),
  ],
  anyWithoutMerchant: false,
);

/// A whole table of expenses, paged by the harness the way the core pages it.
///
/// Built as one list rather than per-page fixtures so the tests can assert that
/// a page is a window onto a stable order — the property paging exists to have.
List<TxnSummary> manyExpenses(int n) => [
      for (int i = 1; i <= n; i++)
        expense(i, merchant: 'Shop $i', total: i * 100),
    ];

class Harness {
  Harness({
    this.rows = const [],
    this.values = someValues,
    this.deleteFails = false,
    this.loadFails = false,
    this.totalsFail = false,
  });

  List<TxnSummary> rows;
  FilterValues values;
  bool deleteFails;
  bool loadFails;
  bool totalsFail;

  final List<TxnQuery> queries = [];
  final List<TxnQuery> totalsQueries = [];
  final List<int> deleted = [];
  int filterValueLoads = 0;

  /// Slices [rows] the way SQL would, so `limit`/`offset` are exercised rather
  /// than assumed: a screen that asked for the right page and then rendered the
  /// wrong one would still pass a stub that ignored them.
  Future<List<TxnSummary>> load(TxnQuery query) async {
    queries.add(query);
    if (loadFails) throw Exception('the database is not open yet');
    if (query.offset >= rows.length) return const [];
    final int end = query.offset + query.limit;
    return rows.sublist(query.offset, end > rows.length ? rows.length : end);
  }

  /// Deliberately ignores `limit` and `offset`, exactly as the core does. The
  /// totals row is the whole filtered set, and a stub that paged them would
  /// hide the bug where the screen expects a page total.
  Future<TxnTotals> loadTotals(TxnQuery query) async {
    totalsQueries.add(query);
    if (totalsFail) throw Exception('the database is not open yet');
    return TxnTotals(
      subtotal: fromCore(rows.length * 100),
      tax: fromCore(rows.length * 7),
      total: fromCore(rows.length * 107),
      expenses: rows.length,
    );
  }

  Future<FilterValues> loadValues() async {
    filterValueLoads++;
    return values;
  }

  Future<void> delete(int id) async {
    if (deleteFails) throw Exception('transaction $id not found');
    deleted.add(id);
    rows = rows.where((r) => r.id != id).toList();
  }

  TxnQuery get lastQuery => queries.last;
}

Future<void> pump(WidgetTester tester, Harness harness,
    {double width = 1000}) async {
  tester.view.physicalSize = Size(width, 900);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);

  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: ExpensesScreen(
          load: harness.load,
          loadTotals: harness.loadTotals,
          loadFilterValues: harness.loadValues,
          delete: harness.delete,
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
}

Future<void> openMenu(WidgetTester tester, String label) async {
  await tester.tap(find.text(label));
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('lists expenses newest first by default', (tester) async {
    final harness = Harness(rows: [expense(1), expense(2, merchant: 'Chevron', total: 6105)]);
    await pump(tester, harness);

    expect(harness.lastQuery.sort, TxnSort.dateDesc);
    expect(harness.lastQuery.limit, 50);
    expect(harness.lastQuery.offset, 0);
    expect(find.text("Macey's"), findsOneWidget);
    expect(find.text(r'$118.40'), findsOneWidget);
  });

  testWidgets('an untouched filter sends null, not a list of everything',
      (tester) async {
    // The two return the same rows today and different rows tomorrow.
    final harness = Harness(rows: [expense(1)]);
    await pump(tester, harness);

    expect(harness.lastQuery.merchantIds, isNull);
    expect(harness.lastQuery.categoryIds, isNull);
    // Untouched also means quick-added expenses are not silently dropped.
    expect(harness.lastQuery.includeNoMerchant, isTrue);
  });

  testWidgets('the date header flips the sort', (tester) async {
    final harness = Harness(rows: [expense(1)]);
    await pump(tester, harness);

    await tester.tap(find.text('Date'));
    await tester.pumpAndSettle();
    expect(harness.lastQuery.sort, TxnSort.dateAsc);

    // A toggle, not a cycle: there is no third unsorted state.
    await tester.tap(find.text('Date'));
    await tester.pumpAndSettle();
    expect(harness.lastQuery.sort, TxnSort.dateDesc);
  });

  testWidgets('ticking a store off narrows to the rest', (tester) async {
    final harness = Harness(rows: [expense(1)]);
    await pump(tester, harness);

    await openMenu(tester, 'Store');
    await tester.tap(find.text('Chevron'));
    await tester.pumpAndSettle();

    expect(harness.lastQuery.merchantIds, [2]);
  });

  testWidgets('hide all asks for nothing, without asking the core',
      (tester) async {
    final harness = Harness(rows: [expense(1)]);
    await pump(tester, harness);
    final before = harness.queries.length;

    await openMenu(tester, 'Store');
    await tester.tap(find.text('Hide all'));
    await tester.pumpAndSettle();

    // The core reads an empty filter set as "no filter", so a hide-all that
    // was sent would show the whole table — the opposite of itself.
    expect(harness.queries.length, before);
    expect(find.text('No expenses match those filters.'), findsOneWidget);
  });

  testWidgets('show all puts the filter back to untouched', (tester) async {
    final harness = Harness(rows: [expense(1)]);
    await pump(tester, harness);

    await openMenu(tester, 'Store');
    await tester.tap(find.text('Hide all'));
    await tester.pumpAndSettle();
    // The menu stays open: every tick reloads the list, and a menu that shut
    // after each one would make "everything except these three" three trips.
    await tester.tap(find.text('Show all'));
    await tester.pumpAndSettle();

    expect(harness.lastQuery.merchantIds, isNull);
  });

  testWidgets('(No store) is a box, not an id', (tester) async {
    final harness = Harness(
      rows: [expense(1, merchant: null)],
      values: const FilterValues(
        merchants: [FilterValue(id: 1, name: 'Chevron')],
        categories: [FilterValue(id: 10, name: 'Gas')],
        anyWithoutMerchant: true,
      ),
    );
    await pump(tester, harness);

    await openMenu(tester, 'Store');
    await tester.tap(find.text('Chevron'));
    await tester.pumpAndSettle();

    // Chevron hidden leaves only the sentinel, which must be folded into the
    // flag rather than parsed as a merchant id.
    expect(harness.lastQuery.merchantIds, isEmpty);
    expect(harness.lastQuery.includeNoMerchant, isTrue);
  });

  testWidgets('an expense with no store shows an em dash', (tester) async {
    final harness = Harness(rows: [expense(1, merchant: null)]);
    await pump(tester, harness);

    expect(find.text('—'), findsOneWidget);
  });

  testWidgets('an empty database says so differently from an empty filter',
      (tester) async {
    final harness = Harness();
    await pump(tester, harness);

    expect(find.textContaining('No expenses yet'), findsOneWidget);
    expect(find.text('No expenses match those filters.'), findsNothing);
  });

  testWidgets('a failed load keeps the rows that were there', (tester) async {
    final harness = Harness(rows: [expense(1)]);
    await pump(tester, harness);
    expect(find.text("Macey's"), findsOneWidget);

    harness.loadFails = true;
    await tester.tap(find.text('Date'));
    await tester.pumpAndSettle();

    expect(find.textContaining('Could not load expenses'), findsOneWidget);
    // Replacing a list you were reading with an error page loses the thing you
    // were reading.
    expect(find.text("Macey's"), findsOneWidget);
  });

  testWidgets('deleting asks first, and names what it would delete',
      (tester) async {
    final harness = Harness(rows: [expense(1)]);
    await pump(tester, harness);

    await tester.tap(find.byTooltip('Delete this expense'));
    await tester.pumpAndSettle();

    expect(find.text('Delete this expense?'), findsOneWidget);
    expect(find.textContaining("2026-08-18 · Macey's · \$118.40"), findsOneWidget);
    expect(find.text('Delete Expense'), findsOneWidget);
    expect(find.text('Cancel'), findsOneWidget);
  });

  testWidgets('cancelling deletes nothing', (tester) async {
    final harness = Harness(rows: [expense(1)]);
    await pump(tester, harness);

    await tester.tap(find.byTooltip('Delete this expense'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();

    expect(harness.deleted, isEmpty);
    expect(find.text("Macey's"), findsOneWidget);
  });

  testWidgets('confirming removes the row in place and refreshes the menus',
      (tester) async {
    final harness = Harness(rows: [expense(1), expense(2, merchant: 'Chevron', total: 6105)]);
    await pump(tester, harness);
    final loadsBefore = harness.queries.length;
    final menuLoadsBefore = harness.filterValueLoads;

    await tester.tap(find.byTooltip('Delete this expense').first);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Delete Expense'));
    await tester.pumpAndSettle();

    expect(harness.deleted, [1]);
    expect(find.text("Macey's"), findsNothing);
    expect(find.text('Chevron'), findsOneWidget);
    // No reload: it would scroll the user back to the top of two hundred rows
    // to show them one that is gone.
    expect(harness.queries.length, loadsBefore);
    // But the menus are re-read, so an emptied shop leaves the menu.
    expect(harness.filterValueLoads, greaterThan(menuLoadsBefore));
  });

  testWidgets('deleting the last row reloads, so the list can say what it is',
      (tester) async {
    final harness = Harness(rows: [expense(1)]);
    await pump(tester, harness);
    final loadsBefore = harness.queries.length;

    await tester.tap(find.byTooltip('Delete this expense'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Delete Expense'));
    await tester.pumpAndSettle();

    expect(harness.queries.length, greaterThan(loadsBefore));
    expect(find.textContaining('No expenses yet'), findsOneWidget);
  });

  testWidgets('a failed delete says so and keeps the row', (tester) async {
    final harness = Harness(rows: [expense(1)], deleteFails: true);
    await pump(tester, harness);

    await tester.tap(find.byTooltip('Delete this expense'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Delete Expense'));
    await tester.pumpAndSettle();

    expect(find.textContaining('Could not delete that expense'), findsOneWidget);
    expect(find.text("Macey's"), findsOneWidget);
  });

  testWidgets('a page is fifty rows, and Next asks for the next fifty',
      (tester) async {
    final harness = Harness(rows: manyExpenses(120));
    await pump(tester, harness);

    expect(find.text('Showing 1\u201350 of 120'), findsOneWidget);

    await tester.tap(find.widgetWithIcon(IconButton, Icons.chevron_right));
    await tester.pumpAndSettle();

    expect(harness.lastQuery.offset, 50);
    expect(harness.lastQuery.limit, 50);
    expect(find.text('Showing 51\u2013100 of 120'), findsOneWidget);

    // The last page is what is left of one, not a full one padded out.
    await tester.tap(find.widgetWithIcon(IconButton, Icons.chevron_right));
    await tester.pumpAndSettle();
    expect(find.text('Showing 101\u2013120 of 120'), findsOneWidget);
  });

  testWidgets('page two does not repeat page one', (tester) async {
    final harness = Harness(rows: manyExpenses(120));
    await pump(tester, harness);
    expect(find.text('Shop 1'), findsOneWidget);

    await tester.tap(find.widgetWithIcon(IconButton, Icons.chevron_right));
    await tester.pumpAndSettle();

    // The offset is the whole of the mechanism, and an off-by-one in it shows
    // one receipt twice and hides another for good — the reader has no way to
    // notice, because both pages still look like pages.
    expect(find.text('Shop 1'), findsNothing);
    expect(find.text('Shop 51'), findsOneWidget);
  });

  testWidgets('the ends of the list disable the control that would leave them',
      (tester) async {
    final harness = Harness(rows: manyExpenses(60));
    await pump(tester, harness);

    IconButton button(IconData icon) =>
        tester.widget<IconButton>(find.widgetWithIcon(IconButton, icon));

    // Disabled rather than hidden: a control that vanishes moves the one next
    // to it under a pointer that was aiming at something else.
    expect(button(Icons.chevron_left).onPressed, isNull);
    expect(button(Icons.chevron_right).onPressed, isNotNull);

    await tester.tap(find.widgetWithIcon(IconButton, Icons.chevron_right));
    await tester.pumpAndSettle();

    expect(button(Icons.chevron_left).onPressed, isNotNull);
    expect(button(Icons.chevron_right).onPressed, isNull);
  });

  testWidgets('the totals row adds up the whole filter, not the page shown',
      (tester) async {
    final harness = Harness(rows: manyExpenses(120));
    await pump(tester, harness);

    // 120 expenses, of which only 50 are on screen. The useful number is what
    // the filter comes to, and a total that changed every time the reader
    // pressed Next would read as an arithmetic bug rather than as a page.
    expect(find.text('Total of all 120 expenses'), findsOneWidget);
    expect(find.text(r'$128.40'), findsOneWidget);

    await tester.tap(find.widgetWithIcon(IconButton, Icons.chevron_right));
    await tester.pumpAndSettle();

    expect(find.text('Total of all 120 expenses'), findsOneWidget);
    expect(find.text(r'$128.40'), findsOneWidget);
    // And the core is asked for the whole set, never for a page of it: the
    // screen must not be able to sum a page even by accident.
    expect(harness.totalsQueries, isNotEmpty);
  });

  testWidgets('changing a filter goes back to the first page', (tester) async {
    final harness = Harness(rows: manyExpenses(120));
    await pump(tester, harness);

    await tester.tap(find.widgetWithIcon(IconButton, Icons.chevron_right));
    await tester.pumpAndSettle();
    expect(harness.lastQuery.offset, 50);

    await openMenu(tester, 'Store');
    await tester.tap(find.text('Chevron'));
    await tester.pumpAndSettle();

    // Otherwise a narrowed filter lands on a page past the end of its own
    // result: an empty screen that is indistinguishable from a filter that
    // matched nothing.
    expect(harness.lastQuery.offset, 0);
    expect(harness.lastQuery.merchantIds, [2]);
  });

  testWidgets('flipping the sort goes back to the first page', (tester) async {
    final harness = Harness(rows: manyExpenses(120));
    await pump(tester, harness);

    await tester.tap(find.widgetWithIcon(IconButton, Icons.chevron_right));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Date'));
    await tester.pumpAndSettle();

    // Page 2 of newest-first and page 2 of oldest-first hold different
    // receipts. Staying put would look like the sort had scrambled the list.
    expect(harness.lastQuery.offset, 0);
    expect(harness.lastQuery.sort, TxnSort.dateAsc);
  });

  testWidgets('an empty result has no pager and no row of zeroes',
      (tester) async {
    final harness = Harness();
    await pump(tester, harness);

    expect(find.widgetWithIcon(IconButton, Icons.chevron_right), findsNothing);
    expect(find.textContaining('Showing'), findsNothing);
    // A total of $0.00 under "no expenses yet" is the same news twice.
    expect(find.textContaining('Total of'), findsNothing);
  });

  testWidgets('deleting refreshes the bottom line without reloading the page',
      (tester) async {
    final harness = Harness(rows: manyExpenses(3));
    await pump(tester, harness);
    final listLoads = harness.queries.length;
    final totalsLoads = harness.totalsQueries.length;

    await tester.tap(find.byTooltip('Delete this expense').first);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Delete Expense'));
    await tester.pumpAndSettle();

    // The rows stay where they are, but the total must not: it sums the whole
    // filter, so until it is asked again it still contains money that is gone,
    // and a bottom line that disagrees with the rows above it is worse than a
    // moment of loading.
    expect(harness.queries.length, listLoads);
    expect(harness.totalsQueries.length, greaterThan(totalsLoads));
    expect(find.text('Total of all 2 expenses'), findsOneWidget);
  });

  testWidgets('a phone gets a list rather than a table', (tester) async {
    final harness = Harness(rows: [expense(1)]);
    await pump(tester, harness, width: 420);

    expect(find.byType(ListTile), findsOneWidget);
    expect(find.text("Macey's"), findsOneWidget);
    // The date moves under the store rather than into a column of its own.
    expect(find.textContaining('2026-08-18'), findsOneWidget);
  });
}
