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

  testWidgets('behind on funding is not over budget', (tester) async {
    await pump(tester, rows: [
      row('Groceries', funding: 'behind', fundingBy: 49537, overspentBy: 0),
    ]);

    // The funding column says it in words; the Over column stays an em dash.
    expect(find.text(r'behind $495.37'), findsOneWidget);
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

  testWidgets('a month with nothing budgeted says how to budget something',
      (tester) async {
    await pump(tester, rows: [unbudgeted]);

    expect(find.textContaining('No budgets set for this month'), findsOneWidget);
    expect(find.textContaining('pacc budget set'), findsOneWidget);
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

    expect(find.textContaining('They disagree by design'), findsOneWidget);
  });
}
