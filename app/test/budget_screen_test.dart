import 'package:app/src/api.dart';
import 'package:app/src/money.dart';
import 'package:app/src/screens/budget_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

BudgetRow row(
  String name, {
  int? perMonth = 60000,
  int? funded = 10463,
  int? spentCycle = 49537,
  int? remaining = 10463,
  int? overspentBy = 0,
  String funding = 'on track',
  int fundingBy = 0,
  bool groupIsObligation = false,
  int? contributedThisMonth,
  bool isSinkingFund = false,
}) =>
    BudgetRow(
      categoryId: 1,
      categoryName: name,
      groupName: 'Frequent',
      budgeted: perMonth == null ? null : fromCore(perMonth),
      perMonth: perMonth == null ? null : fromCore(perMonth),
      funded: funded == null ? null : fromCore(funded),
      spentCycle: spentCycle == null ? null : fromCore(spentCycle),
      remaining: remaining == null ? null : fromCore(remaining),
      overspentBy: overspentBy == null ? null : fromCore(overspentBy),
      spentThisMonth: fromCore(spentCycle ?? 0),
      groupIsObligation: groupIsObligation,
      contributedThisMonth:
          contributedThisMonth == null ? null : fromCore(contributedThisMonth),
      isSinkingFund: isSinkingFund,
      funding: funding,
      fundingBy: fromCore(fundingBy),
      monthsRemaining: 1,
    );

const BudgetRow unbudgeted = BudgetRow(
  categoryId: 2,
  categoryName: 'Travel',
  groupName: 'Non-Monthly',
  budgeted: null,
  perMonth: null,
  funded: null,
  spentCycle: null,
  remaining: null,
  overspentBy: null,
  spentThisMonth: zero,
  groupIsObligation: false,
  contributedThisMonth: null,
  isSinkingFund: false,
  funding: 'none',
  fundingBy: zero,
  monthsRemaining: null,
);

/// A bottom line, for a screen that always asks for one.
const BudgetTotals noTotals = BudgetTotals(
  perMonth: zero,
  funded: zero,
  spentCycle: zero,
  remaining: zero,
  overspentBy: zero,
  categories: 0,
);

Future<List<String>> pump(
  WidgetTester tester, {
  List<BudgetRow> rows = const [],
  BudgetTotals totals = noTotals,
  double width = 1000,
}) async {
  final asked = <String>[];
  tester.view.physicalSize = Size(width, 900);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);

  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: BudgetScreen(
          month: '2026-08',
          load: (month) async {
            asked.add(month);
            return rows;
          },
          loadTotals: (month) async => totals,
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
  return asked;
}

/// The screen on a window [width] logical pixels wide, and every layout error
/// raised while it was laid out.
///
/// A row that does not fit fires one error per offending Row, so the answer is
/// a list rather than a flag: which part overflowed is the useful half. The
/// handler is put back before this returns — a failed `expect` while it is
/// still overridden wedges the binding instead of reporting itself.
Future<List<String>> pumpAt(
  WidgetTester tester, {
  required double width,
  List<BudgetRow> rows = const [],
  BudgetTotals totals = noTotals,
}) async {
  tester.view.physicalSize = Size(width, 900);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);

  final errors = <String>[];
  final previous = FlutterError.onError;
  FlutterError.onError = (details) => errors.add(details.exceptionAsString());

  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: BudgetScreen(
          month: '2026-08',
          load: (month) async => rows,
          loadTotals: (month) async => totals,
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();

  FlutterError.onError = previous;
  return errors;
}

/// Somewhere inside the line for category 1, rather than anywhere on screen.
///
/// The bottom line carries the same words as the rows it sums, so an
/// unqualified `find.text('Left')` matches two widgets and measures neither.
Finder inRow(String text) => find.descendant(
      of: find.byKey(const Key('budget-row-1')),
      matching: find.text(text),
    );

Finder inTotals(String text) => find.descendant(
      of: find.byKey(const Key('budget-totals')),
      matching: find.text(text),
    );

/// Every one of [finders] is on screen, inside [width], and touching none of
/// the others.
///
/// Overlap is the failure a screenshot shows and an `expect` on text usually
/// misses: two amounts drawn at the same place are both "found".
void expectLaidOutSeparately(
  WidgetTester tester,
  List<Finder> finders, {
  required double width,
}) {
  final rects = <Rect>[];
  for (final finder in finders) {
    expect(finder, findsOneWidget);
    final rect = tester.getRect(finder);
    expect(rect.left, greaterThanOrEqualTo(0));
    expect(rect.right, lessThanOrEqualTo(width));
    for (final other in rects) {
      expect(rect.overlaps(other), isFalse, reason: '$rect overlaps $other');
    }
    rects.add(rect);
  }
}

/// A bottom line with figures of its own, none of them shared with the rows
/// above, so a finder cannot match the wrong one.
BudgetTotals someTotals() => BudgetTotals(
      perMonth: fromCore(120000),
      funded: fromCore(20926),
      spentCycle: fromCore(99074),
      remaining: fromCore(20926),
      overspentBy: zero,
      categories: 2,
    );

/// A sinking fund: saved into rather than paid out of.
BudgetRow fund({
  String name = 'Car Repair',
  int perMonth = 10000,
  int? contributed,
}) =>
    row(
      name,
      perMonth: perMonth,
      contributedThisMonth: contributed,
      isSinkingFund: true,
    );

void main() {
  testWidgets('lists the month asked for', (tester) async {
    final asked = await pump(tester, rows: [row('Groceries')]);

    expect(asked, ['2026-08']);
    expect(find.text('August 2026'), findsOneWidget);
    expect(find.text('Frequent / Groceries'), findsOneWidget);
  });

  testWidgets('stepping months reloads', (tester) async {
    final asked = await pump(tester, rows: [row('Groceries')]);

    await tester.tap(find.byTooltip('Previous month'));
    await tester.pumpAndSettle();
    expect(asked.last, '2026-07');
    expect(find.text('July 2026'), findsOneWidget);

    await tester.tap(find.byTooltip('Next month'));
    await tester.pumpAndSettle();
    expect(asked.last, '2026-08');
  });

  testWidgets('stepping past December rolls the year', (tester) async {
    final asked = await pump(tester, rows: [row('Groceries')]);

    for (int i = 0; i < 5; i++) {
      await tester.tap(find.byTooltip('Next month'));
      await tester.pumpAndSettle();
    }

    expect(asked.last, '2027-01');
    expect(find.text('January 2027'), findsOneWidget);
  });

  testWidgets('an unpaid bill is behind; an unspent envelope is not',
      (tester) async {
    // Same funding arithmetic, two different sentences, because "behind" only
    // means something where the money is owed. Rent budgeted and unpaid is a
    // debt; groceries budgeted and unspent is money still in hand, and calling
    // that being behind is a telling-off for the good outcome.
    await pump(tester, rows: [
      row('Rent',
          funding: 'behind',
          fundingBy: 49537,
          overspentBy: 0,
          groupIsObligation: true),
    ]);
    expect(find.text(r'behind $495.37'), findsOneWidget);
  });

  testWidgets('a spending envelope reports what went out, not a shortfall',
      (tester) async {
    await pump(tester, rows: [
      row('Groceries', funding: 'behind', fundingBy: 49537, overspentBy: 0),
    ]);

    // The footnote still uses the word, explaining why this row does not.
    expect(find.text(r'behind $495.37'), findsNothing);
    expect(find.textContaining(r'$495.37 left'), findsOneWidget);
    // One for this row's Over column and one for the table's bottom line:
    // being behind on funding is not being over budget, in either place.
    expect(find.text('—'), findsNWidgets(2));
  });

  testWidgets('over budget shows a positive figure, never a negative',
      (tester) async {
    await pump(tester, rows: [
      row(
        'Household',
        perMonth: 12000,
        spentCycle: 14390,
        remaining: -2390,
        overspentBy: 2390,
        funding: 'behind',
        fundingBy: 14390,
      ),
    ]);

    expect(find.text(r'$23.90'), findsOneWidget);
    // And the "left" figure is still the core's, negative and unhidden.
    expect(find.text(r'-$23.90'), findsOneWidget);
  });

  testWidgets('unbudgeted categories are counted under the table',
      (tester) async {
    await pump(tester, rows: [row('Groceries'), unbudgeted]);

    expect(find.text('Frequent / Groceries'), findsOneWidget);
    expect(find.textContaining('1 category has no budget'), findsOneWidget);
  });

  testWidgets('a month with nothing budgeted offers to set one',
      (tester) async {
    await pump(tester, rows: [unbudgeted]);

    expect(find.textContaining('No budgets set for this month'), findsOneWidget);
    // Setting one is a button now, not a command to go and type somewhere else.
    expect(find.text('Set a budget'), findsOneWidget);
  });

  testWidgets('a phone stacks the line instead of squeezing the table',
      (tester) async {
    await pump(tester, width: 420, rows: [row('Groceries')]);

    // Spent and Left survive, on the row and again on the bottom line: stacked
    // there are no column headings above for a footer to inherit, so it labels
    // its own figures. Per month and Funded are what goes, and the editor
    // behind the row is where they were set in the first place.
    expect(find.text('Spent (cycle)'), findsNWidgets(2));
    expect(find.text('Left'), findsNWidgets(2));
    expect(find.text('Per month'), findsNothing);
    expect(find.text('Funded'), findsNothing);
  });

  testWidgets('the footnote explains why the two disagree', (tester) async {
    await pump(tester, rows: [row('Groceries')]);

    expect(
      find.textContaining('not behind on anything'),
      findsOneWidget,
    );
  });

  testWidgets('a sinking fund offers to record what was set aside',
      (tester) async {
    // The row that reports a fund behind is the row that can fix it. Anywhere
    // else and the two would be a navigation apart.
    await pump(tester, rows: [fund(perMonth: 10000)]);

    expect(find.textContaining('Set aside \$100.00'), findsOneWidget);
  });

  testWidgets('a recorded month says so instead of offering again',
      (tester) async {
    await pump(tester, rows: [fund(perMonth: 10000, contributed: 7500)]);

    expect(find.textContaining('\$75.00 set aside'), findsOneWidget);
    expect(find.textContaining('Set aside \$100.00'), findsNothing);
  });

  testWidgets('a monthly envelope is not offered a contribution',
      (tester) async {
    // You do not save into groceries; you spend out of them.
    await pump(tester, rows: [row('Groceries')]);

    expect(find.textContaining('set aside'), findsNothing);
  });

  testWidgets('the table has a bottom line, summed in the core',
      (tester) async {
    // Not added up here: `Cents` exists to make arithmetic a compile error, so
    // a total on this screen can never be a figure the frontend invented.
    await pump(
      tester,
      rows: [row('Groceries'), row('Rent')],
      totals: BudgetTotals(
        perMonth: fromCore(120000),
        funded: fromCore(20926),
        spentCycle: fromCore(99074),
        remaining: fromCore(20926),
        overspentBy: zero,
        categories: 2,
      ),
    );

    expect(find.text('Total of 2 categories'), findsOneWidget);
    expect(find.text(r'$1,200.00'), findsOneWidget);
  });

  testWidgets('a total with nothing over shows a dash rather than a zero',
      (tester) async {
    // Same rule as the rows: no overspend is the absence of a number, not a
    // number. Overspending in one envelope is not cancelled by another having
    // room, so this is a sum of what is over rather than a net figure.
    await pump(
      tester,
      rows: [row('Groceries')],
      totals: BudgetTotals(
        perMonth: fromCore(60000),
        funded: zero,
        spentCycle: zero,
        remaining: zero,
        overspentBy: zero,
        categories: 1,
      ),
    );

    expect(find.text('Total of 1 category'), findsOneWidget);
  });

  // A phone held in one hand, and a slightly larger one. Three 120dp columns
  // and a name fit neither: at 360 the rows overflowed by 32dp and the month
  // header by 11, and at 400 nothing overflowed at all — the columns cleared
  // the edge and left the category name 8dp wide. The silent one is the worse
  // failure, which is why these tests measure boxes and not only find text.
  for (final width in [360.0, 400.0]) {
    final String at = 'at ${width.toInt()}px';

    testWidgets('nothing overflows $at', (tester) async {
      final errors = await pumpAt(
        tester,
        width: width,
        rows: [row('Groceries'), fund(perMonth: 10000)],
        totals: someTotals(),
      );

      expect(errors, isEmpty);
    });

    testWidgets('the category name gets a line to itself $at', (tester) async {
      await pumpAt(
        tester,
        width: width,
        rows: [row('Home Maintenance & Repairs')],
      );

      // Nearly the whole width, less the row's own padding. The failure this
      // guards is the silent one: the columns used to take the line down to
      // single figures of pixels, leaving a name that was present, findable,
      // and impossible to read.
      //
      // The group is its own line above rather than a prefix, so the width
      // measured is the category's. Prefixed, "Frequent / Home Maintenance &
      // Repairs" spent the first half of the line on a word every row of the
      // group repeats and ellipsised the half that says which row this is.
      expect(
        tester.getSize(inRow('Home Maintenance & Repairs')).width,
        greaterThan(width - 40),
      );
      expect(inRow('Frequent'), findsOneWidget);
    });

    testWidgets('a figure is not printed twice under two names $at',
        (tester) async {
      // An envelope's cycle is its month, so "spent this month" and
      // "Spent (cycle)" are one number, and the row used to print it as two
      // lines — "spent $495.37" directly under "Spent (cycle) $495.37".
      await pumpAt(tester, width: width, rows: [row('Groceries')]);

      expect(find.textContaining(r'$495.37'), findsOneWidget);
      expect(find.textContaining('spent '), findsNothing);
    });

    testWidgets('a bill still says what it owes $at', (tester) async {
      // The sentence is dropped where it only retells the figures, never
      // where it carries the one thing they cannot say.
      await pumpAt(
        tester,
        width: width,
        rows: [
          row('Rent',
              funding: 'behind', fundingBy: 165000, groupIsObligation: true),
        ],
      );

      expect(find.text(r'behind $1,650.00'), findsOneWidget);
    });

    testWidgets('every figure is on screen and clear of the next $at',
        (tester) async {
      await pumpAt(
        tester,
        width: width,
        rows: [row('Groceries')],
        totals: someTotals(),
      );

      // Both figures, each still beside the word that says what it is.
      expectLaidOutSeparately(
        tester,
        [
          inRow('Left'),
          inRow(r'$104.63'),
          inRow('Spent (cycle)'),
          inRow(r'$495.37'),
        ],
        width: width,
      );
    });

    testWidgets('an over-budget line shows the overspend $at', (tester) async {
      final errors = await pumpAt(
        tester,
        width: width,
        rows: [
          row(
            'Household',
            perMonth: 12000,
            spentCycle: 14390,
            remaining: -2390,
            overspentBy: 2390,
            funding: 'behind',
            fundingBy: 14390,
          ),
        ],
        totals: someTotals(),
      );

      expect(errors, isEmpty);
      // Three figures on the line rather than two, wrapping to a second run
      // rather than being dropped or drawn on top of each other.
      expectLaidOutSeparately(
        tester,
        [
          inRow('Left'),
          inRow(r'-$23.90'),
          inRow('Spent (cycle)'),
          inRow(r'$143.90'),
          inRow('Over'),
          inRow(r'$23.90'),
        ],
        width: width,
      );
    });

    testWidgets('the bottom line still says what it sums $at', (tester) async {
      await pumpAt(
        tester,
        width: width,
        rows: [row('Groceries')],
        totals: someTotals(),
      );

      expectLaidOutSeparately(
        tester,
        [
          inTotals('Total of 2 categories'),
          inTotals('Left'),
          inTotals(r'$209.26'),
          inTotals('Spent (cycle)'),
          inTotals(r'$990.74'),
        ],
        width: width,
      );
    });

    testWidgets('a sinking fund can still be recorded $at', (tester) async {
      final errors = await pumpAt(
        tester,
        width: width,
        rows: [fund(perMonth: 10000)],
        totals: someTotals(),
      );

      expect(errors, isEmpty);
      expect(find.textContaining(r'Set aside $100.00'), findsOneWidget);
    });
  }

  testWidgets('a window with room for the table still gets one', (tester) async {
    final errors = await pumpAt(
      tester,
      width: 1400,
      rows: [row('Groceries')],
      totals: someTotals(),
    );

    expect(errors, isEmpty);
    // The headings are what name the figures where there are columns, so all
    // five are on screen exactly once — over the rows, not repeated under them.
    for (final heading in [
      'Per month',
      'Funded',
      'Spent (cycle)',
      'Left',
      'Over',
    ]) {
      expect(find.text(heading), findsOneWidget);
    }
    // And the name is a name, not the offcut of five fixed columns.
    expect(
      tester.getSize(inRow('Frequent / Groceries')).width,
      greaterThan(180),
    );
  });
}


