import 'package:app/src/api.dart';
import 'package:app/src/money.dart';
import 'package:app/src/screens/expenses_screen.dart';
import 'package:app/src/shell.dart';
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

/// A month's worth of expenses, all on the same day.
///
/// Built as one list rather than per-screen fixtures so a test can assert that
/// the whole month is on screen — the property the month control exists to
/// have, now that nothing else pages the list.
List<TxnSummary> manyExpenses(int n, {String date = '2026-08-18'}) => [
      for (int i = 1; i <= n; i++)
        expense(i, merchant: 'Shop $i', total: i * 100, date: date),
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

  /// The rows the query's date range actually asks for.
  ///
  /// ISO dates compare as strings exactly as they compare as dates, which is
  /// the whole reason the core stores them that way. Applied here rather than
  /// ignored so a screen that asked for the right month and rendered another
  /// fails, instead of passing against a stub that never looked at the bounds.
  List<TxnSummary> within(TxnQuery query) => rows
      .where((r) =>
          (query.from == null || r.txnDate.compareTo(query.from!) >= 0) &&
          (query.to == null || r.txnDate.compareTo(query.to!) <= 0))
      .toList();

  /// Slices the matching rows the way SQL would, so `limit` is exercised
  /// rather than assumed: a screen that rendered more rows than it asked for
  /// would still pass a stub that ignored it.
  Future<List<TxnSummary>> load(TxnQuery query) async {
    queries.add(query);
    if (loadFails) throw Exception('the database is not open yet');
    return within(query).skip(query.offset).take(query.limit).toList();
  }

  /// Deliberately ignores `limit` and `offset`, exactly as the core does, and
  /// deliberately honours the date range, exactly as the core does. The totals
  /// row is the whole month, not the rows drawn, and a stub that confused the
  /// two would hide both bugs.
  Future<TxnTotals> loadTotals(TxnQuery query) async {
    totalsQueries.add(query);
    if (totalsFail) throw Exception('the database is not open yet');
    final int n = within(query).length;
    return TxnTotals(
      subtotal: fromCore(n * 100),
      tax: fromCore(n * 7),
      total: fromCore(n * 107),
      expenses: n,
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

/// The month every fixture sits in.
///
/// Pinned rather than left to the clock: a screen that opens on the real
/// current month would make every assertion here expire, and the failure would
/// arrive one September morning with nothing to blame it on.
const String pinnedMonth = '2026-08';

Future<void> pump(
  WidgetTester tester,
  Harness harness, {
  double width = 1000,
  String month = pinnedMonth,
}) async {
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
          month: month,
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
}

/// The expense list inside the real navigation shell, which is where the
/// floating button that used to sit on top of its totals comes from.
Future<void> pumpInShell(
  WidgetTester tester,
  Harness harness, {
  required double width,
  double textScale = 1.0,
}) async {
  tester.view.physicalSize = Size(width, 900);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);

  await tester.pumpWidget(
    MaterialApp(
      builder: (context, child) => MediaQuery.withClampedTextScaling(
        minScaleFactor: textScale,
        maxScaleFactor: textScale,
        child: child!,
      ),
      home: AppShell(
        // Every destination draws the expense list: what is under test is the
        // chrome the shell puts around it, not which destination is selected.
        screenBuilder: (context, destination) => ExpensesScreen(
          load: harness.load,
          loadTotals: harness.loadTotals,
          loadFilterValues: harness.loadValues,
          delete: harness.delete,
          month: pinnedMonth,
        ),
        onNewExpense: (context) {},
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
    // The month is the whole of the range asked for, inclusive at both ends.
    expect(harness.lastQuery.from, '2026-08-01');
    expect(harness.lastQuery.to, '2026-08-31');
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
    expect(
      find.text('No expenses match those filters in August 2026.'),
      findsOneWidget,
    );
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

  testWidgets('an empty month says so differently from an empty filter',
      (tester) async {
    final harness = Harness();
    await pump(tester, harness);

    expect(find.text('Nothing recorded in August 2026.'), findsOneWidget);
    expect(
      find.text('No expenses match those filters in August 2026.'),
      findsNothing,
    );
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
    expect(find.text('Nothing recorded in August 2026.'), findsOneWidget);
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

  testWidgets('the month is the paging: all of it is on screen at once',
      (tester) async {
    final harness = Harness(rows: manyExpenses(120));
    await pump(tester, harness);

    // No page controls left to press, and no offset asked for: what used to be
    // "the next fifty" is now "the next month".
    expect(harness.lastQuery.offset, 0);
    expect(find.textContaining('Showing'), findsNothing);
    expect(find.text('Total of all 120 expenses in August 2026'), findsOneWidget);
  });

  testWidgets('stepping back asks for the previous month', (tester) async {
    final harness = Harness(rows: [
      expense(1, date: '2026-08-18'),
      expense(2, merchant: 'Chevron', date: '2026-07-04'),
    ]);
    await pump(tester, harness);
    expect(find.text('August 2026'), findsOneWidget);

    await tester.tap(find.byTooltip('Previous month'));
    await tester.pumpAndSettle();

    expect(harness.lastQuery.from, '2026-07-01');
    expect(harness.lastQuery.to, '2026-07-31');
    expect(find.text('July 2026'), findsOneWidget);
    // And the bounds are what decides the rows, not the label over them.
    expect(find.text('Chevron'), findsOneWidget);
    expect(find.text("Macey's"), findsNothing);

    await tester.tap(find.byTooltip('Next month'));
    await tester.pumpAndSettle();
    expect(harness.lastQuery.from, '2026-08-01');
    expect(find.text("Macey's"), findsOneWidget);
  });

  testWidgets('stepping past January rolls the year', (tester) async {
    final harness = Harness(rows: [expense(1)]);
    await pump(tester, harness, month: '2026-01');

    await tester.tap(find.byTooltip('Previous month'));
    await tester.pumpAndSettle();

    // Month zero is not a month. The rolling is the calendar's, so the reader
    // cannot step into a range the core would refuse.
    expect(find.text('December 2025'), findsOneWidget);
    expect(harness.lastQuery.from, '2025-12-01');
    expect(harness.lastQuery.to, '2025-12-31');
  });

  testWidgets('a short month ends on the day it ends on', (tester) async {
    final harness = Harness(rows: [expense(1, date: '2028-02-29')]);
    await pump(tester, harness, month: '2028-02');

    // A leap day is exactly the receipt a hardcoded 28 or 30 would drop out of
    // a total still calling itself February's.
    expect(harness.lastQuery.to, '2028-02-29');
    expect(find.text('Total of 1 expense in February 2028'), findsOneWidget);
  });

  testWidgets('the month steps in both directions, however empty',
      (tester) async {
    final harness = Harness();
    await pump(tester, harness);

    // Unlike the pager it replaced, neither control is ever a dead end: there
    // is always a month before and after this one, and a disabled arrow would
    // strand a reader in an empty month.
    IconButton button(IconData icon) =>
        tester.widget<IconButton>(find.widgetWithIcon(IconButton, icon));
    expect(button(Icons.chevron_left).onPressed, isNotNull);
    expect(button(Icons.chevron_right).onPressed, isNotNull);
  });

  testWidgets('a month too big to draw says so rather than truncating quietly',
      (tester) async {
    final harness = Harness(rows: manyExpenses(600));
    await pump(tester, harness);

    expect(harness.lastQuery.limit, 500);
    expect(
      find.text('Showing the first 500 of 600 expenses in August 2026.'),
      findsOneWidget,
    );
    // The bottom line still covers the month, which is exactly why the line
    // above it has to exist: the rows and the figure disagree, and the screen
    // is the only thing that can say why.
    expect(find.text('Total of all 600 expenses in August 2026'), findsOneWidget);
  });

  testWidgets('the totals row covers the month, not the whole database',
      (tester) async {
    final harness = Harness(rows: [
      ...manyExpenses(3),
      expense(90, date: '2026-07-31'),
      expense(91, date: '2026-09-01'),
    ]);
    await pump(tester, harness);

    // The day either side of the month is the whole test: an off-by-one at
    // either bound puts a receipt in a month it does not belong to, and the
    // reader has no way to see that it did.
    expect(find.text('Total of all 3 expenses in August 2026'), findsOneWidget);
    expect(find.text(r'$3.21'), findsOneWidget);
    expect(harness.totalsQueries.last.from, '2026-08-01');
    expect(harness.totalsQueries.last.to, '2026-08-31');

    await tester.tap(find.byTooltip('Previous month'));
    await tester.pumpAndSettle();

    expect(find.text('Total of 1 expense in July 2026'), findsOneWidget);
  });

  testWidgets('changing a filter keeps the month it was changed in',
      (tester) async {
    final harness = Harness(rows: manyExpenses(3));
    await pump(tester, harness);

    await tester.tap(find.byTooltip('Previous month'));
    await tester.pumpAndSettle();

    await openMenu(tester, 'Store');
    await tester.tap(find.text('Chevron'));
    await tester.pumpAndSettle();

    // Narrowing to one shop is not a reason to be thrown back to today:
    // comparing two shops over a month would otherwise be impossible.
    expect(harness.lastQuery.from, '2026-07-01');
    expect(harness.lastQuery.merchantIds, [2]);
    expect(find.text('July 2026'), findsOneWidget);
  });

  testWidgets('flipping the sort keeps the month too', (tester) async {
    final harness = Harness(rows: manyExpenses(3));
    await pump(tester, harness);

    await tester.tap(find.byTooltip('Previous month'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Date'));
    await tester.pumpAndSettle();

    expect(harness.lastQuery.sort, TxnSort.dateAsc);
    expect(harness.lastQuery.from, '2026-07-01');
  });

  testWidgets('an empty month has no row of zeroes', (tester) async {
    final harness = Harness();
    await pump(tester, harness);

    // A total of \$0.00 under "nothing recorded" is the same news twice.
    expect(find.textContaining('Total of'), findsNothing);
    expect(find.byKey(const Key('expense-totals')), findsNothing);
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
    expect(find.text('Total of all 2 expenses in August 2026'), findsOneWidget);
  });

  testWidgets('a phone gets a list rather than a table', (tester) async {
    final harness = Harness(rows: [expense(1)]);
    await pump(tester, harness, width: 420);

    expect(find.byType(ListTile), findsOneWidget);
    expect(find.text("Macey's"), findsOneWidget);
    // The date moves under the store rather than into a column of its own.
    expect(find.textContaining('2026-08-18'), findsOneWidget);
  });

  testWidgets('the new-expense button does not sit on the bottom line',
      (tester) async {
    final harness = Harness(rows: [expense(1)]);
    await pumpInShell(tester, harness, width: 1400);

    // The complaint itself, in rectangles: at desktop widths the shell floats
    // an extended button over the content, and it landed on the one row of the
    // screen that cannot be scrolled out from under it.
    final Rect fab = tester.getRect(find.byType(FloatingActionButton));
    final Rect totals = tester.getRect(find.byKey(const Key('expense-totals')));
    expect(fab.overlaps(totals), isFalse);
    // And the month control at the other end of the screen is no nearer it.
    expect(
      fab.overlaps(tester.getRect(find.text('August 2026'))),
      isFalse,
    );
  });

  testWidgets("a phone's bottom line is clear of its button as well",
      (tester) async {
    final harness = Harness(rows: [expense(1)]);
    await pumpInShell(tester, harness, width: 400);

    // The report was about desktop, but the round button covers the money
    // column of a phone's totals row for the same reason and by 48 of the 64
    // pixels it is tall.
    final Rect fab = tester.getRect(find.byType(FloatingActionButton));
    final Rect totals = tester.getRect(find.byKey(const Key('expense-totals')));
    expect(fab.overlaps(totals), isFalse);
    // Whole rather than clipped, and with the money still readable: reserving
    // space is only a fix if the row it protects survives it.
    expect(totals.bottom, lessThanOrEqualTo(tester.view.physicalSize.height));
    expect(find.text(r'$1.07'), findsOneWidget);
  });

  testWidgets('the bottom line is below the button, not above it',
      (tester) async {
    final harness = Harness(rows: [expense(1)]);

    // The button is lifted over the bottom line rather than the bottom line
    // being pushed up off the edge to clear it. What used to sit under the
    // total was a 72dp strip of nothing with a button floating in it, and the
    // month's figure ended one button's height short of the screen edge.
    for (final double width in [400.0, 1400.0]) {
      await pumpInShell(tester, harness, width: width);
      final Rect fab = tester.getRect(find.byType(FloatingActionButton));
      final Rect totals =
          tester.getRect(find.byKey(const Key('expense-totals')));
      expect(fab.bottom, lessThanOrEqualTo(totals.top));
    }
  });

  testWidgets('the bottom line ends where the content ends', (tester) async {
    final harness = Harness(rows: [expense(1)]);

    // Flush with the bottom of the content, which is the point of the
    // rearrangement: a bottom line is only a bottom line if it is at the
    // bottom. On a phone the content stops at the navigation bar.
    await pumpInShell(tester, harness, width: 400);
    expect(
      tester.getRect(find.byKey(const Key('expense-totals'))).bottom,
      moreOrLessEquals(tester.getRect(find.byType(NavigationBar)).top,
          epsilon: 1),
    );

    // Wider there is a drawer beside the content rather than a bar under it,
    // so the bottom line runs to the bottom of the window.
    await pumpInShell(tester, harness, width: 1400);
    expect(
      tester.getRect(find.byKey(const Key('expense-totals'))).bottom,
      moreOrLessEquals(900, epsilon: 1),
    );
  });

  testWidgets('a taller bottom line lifts the button further', (tester) async {
    final harness = Harness(rows: [expense(1)]);

    await pumpInShell(tester, harness, width: 400);
    final double plain =
        tester.getRect(find.byType(FloatingActionButton)).bottom;

    // The lift is measured, not a constant, so a bottom line that wraps to two
    // lines at a larger text size moves the button by what it actually grew
    // rather than by what someone guessed it would.
    await pumpInShell(tester, harness, width: 400, textScale: 2.0);
    final double scaled =
        tester.getRect(find.byType(FloatingActionButton)).bottom;

    expect(scaled, lessThan(plain));
    final Rect fab = tester.getRect(find.byType(FloatingActionButton));
    final Rect totals = tester.getRect(find.byKey(const Key('expense-totals')));
    expect(fab.bottom, lessThanOrEqualTo(totals.top));
  });

  testWidgets('a rail keeps the button, so nothing is reserved under the list',
      (tester) async {
    final harness = Harness(rows: [expense(1)]);
    await pumpInShell(tester, harness, width: 700);

    // At medium widths the button is in the navigation rail. Reserving space
    // for it at the bottom of this screen would be a strip of nothing.
    final Rect fab = tester.getRect(find.byType(FloatingActionButton));
    final Rect totals = tester.getRect(find.byKey(const Key('expense-totals')));
    expect(fab.overlaps(totals), isFalse);
    expect(totals.bottom, tester.view.physicalSize.height);
  });
}
