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

Future<List<String>> pump(
  WidgetTester tester, {
  List<BudgetRow> rows = const [],
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
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
  return asked;
}

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
    expect(find.text('—'), findsOneWidget);
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

  testWidgets('a phone drops the columns the width cannot hold', (tester) async {
    await pump(tester, width: 420, rows: [row('Groceries')]);

    // Spent, Left and Over survive; Per month and Funded are what goes.
    expect(find.text('Spent (cycle)'), findsOneWidget);
    expect(find.text('Left'), findsOneWidget);
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
}

