import 'package:app/src/api.dart';
import 'package:app/src/money.dart';
import 'package:app/src/screens/home_screen.dart';
import 'package:app/src/widgets/budget_ring.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

BudgetRow budgeted(
  String name, {
  required int budget,
  required int spent,
  required int remaining,
  int overspentBy = 0,
  String funding = 'on track',
  int fundingBy = 0,
  int monthsRemaining = 1,
}) =>
    BudgetRow(
      categoryId: 1,
      categoryName: name,
      groupName: 'Frequent',
      budgeted: fromCore(budget),
      perMonth: fromCore(budget),
      funded: fromCore(remaining),
      spentCycle: fromCore(spent),
      remaining: fromCore(remaining),
      overspentBy: fromCore(overspentBy),
      spentThisMonth: fromCore(spent),
      funding: funding,
      fundingBy: fromCore(fundingBy),
      monthsRemaining: monthsRemaining,
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

TxnSummary expense(String? merchant, int total, {String date = '2026-08-19'}) =>
    TxnSummary(
      id: 1,
      txnDate: date,
      merchantName: merchant,
      total: fromCore(total),
      taxIsEstimated: false,
      needsReview: false,
      categories: const ['Groceries'],
    );

/// The legend row for a category. Scoped, because once a category is selected
/// its name is also in the middle of the ring.
Finder legendRow(String name) => find.descendant(
      of: find.byType(InkWell),
      matching: find.text(name),
    );

Future<void> pump(
  WidgetTester tester, {
  List<BudgetRow> budget = const [],
  List<TxnSummary> recent = const [],
  MonthTotals totals = const MonthTotals(budgeted: zero, spent: zero),
  double width = 1000,
}) async {
  tester.view.physicalSize = Size(width, 900);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);

  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: HomeScreen(
          month: '2026-08',
          loadBudget: (month) async => budget,
          loadTotals: (month) async => totals,
          loadRecent: (limit) async => recent.take(limit).toList(),
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('says how much is left of a budget', (tester) async {
    await pump(tester, budget: [
      budgeted('Groceries', budget: 60000, spent: 49537, remaining: 10463),
    ]);

    expect(find.text(r'$104.63'), findsOneWidget);
    expect(find.text('left'), findsOneWidget);
    expect(find.text('over'), findsNothing);
  });

  testWidgets('says how far over, in the error colour', (tester) async {
    await pump(tester, budget: [
      budgeted(
        'Household',
        budget: 12000,
        spent: 14390,
        remaining: -2390,
        overspentBy: 2390,
      ),
    ]);

    expect(find.text(r'$23.90'), findsOneWidget);
    expect(find.text('over'), findsOneWidget);
    expect(find.text('left'), findsNothing);

    final BuildContext context = tester.element(find.text('over'));
    final Color expected = Theme.of(context).colorScheme.error;
    expect(tester.widget<Text>(find.text('over')).style?.color, expected);
  });

  testWidgets('behind on funding is not over budget', (tester) async {
    // The distinction the core insists on: an envelope spent exactly to plan
    // is behind on funding and perfectly fine on spending. Calling that "over"
    // would flag every well-run category.
    await pump(tester, budget: [
      budgeted(
        'Groceries',
        budget: 60000,
        spent: 49537,
        remaining: 10463,
        funding: 'behind',
        fundingBy: 49537,
      ),
    ]);

    expect(find.text('over'), findsNothing);
    expect(find.text('left'), findsOneWidget);
    expect(find.textContaining(r'behind $495.37'), findsOneWidget);
  });

  testWidgets('a longer cycle says so, so "left" is not misread', (tester) async {
    await pump(tester, budget: [
      budgeted(
        'Car Insurance',
        budget: 78000,
        spent: 0,
        remaining: 78000,
        monthsRemaining: 12,
      ),
    ]);

    expect(find.textContaining('12 mo cycle'), findsOneWidget);
  });

  testWidgets('the middle leads with spent out of everything budgeted',
      (tester) async {
    // $200 of groceries and $300 of gas is $500 budgeted, whatever the ring
    // looks like.
    await pump(
      tester,
      budget: [
        budgeted('Groceries', budget: 20000, spent: 7550, remaining: 12450),
        budgeted('Gas', budget: 30000, spent: 4200, remaining: 25800),
      ],
      totals: MonthTotals(budgeted: fromCore(50000), spent: fromCore(11750)),
    );

    expect(find.text(r'$117.50'), findsOneWidget);
    expect(find.text(r'of $500.00'), findsOneWidget);
    expect(find.text('Spent'), findsOneWidget);
  });

  testWidgets('the ring has one segment per budgeted category', (tester) async {
    await pump(tester, budget: [
      budgeted('Groceries', budget: 20000, spent: 7550, remaining: 12450),
      budgeted('Gas', budget: 30000, spent: 4200, remaining: 25800),
      unbudgeted,
    ]);

    final ring = tester.widget<BudgetRing>(find.byType(BudgetRing));
    expect(ring.segments.map((s) => s.label), ['Groceries', 'Gas']);
    // Sized by the month's share, so a yearly line does not swamp the circle.
    expect(ring.segments.map((s) => s.budgeted), [20000, 30000]);
  });

  testWidgets('an over-budget segment is marked over, not merely full',
      (tester) async {
    await pump(tester, budget: [
      budgeted(
        'Household',
        budget: 12000,
        spent: 14390,
        remaining: -2390,
        overspentBy: 2390,
      ),
    ]);

    final ring = tester.widget<BudgetRing>(find.byType(BudgetRing));
    expect(ring.segments.single.isOver, isTrue);
  });

  testWidgets('behind on funding does not mark a segment over', (tester) async {
    await pump(tester, budget: [
      budgeted(
        'Groceries',
        budget: 60000,
        spent: 49537,
        remaining: 10463,
        funding: 'behind',
        fundingBy: 49537,
      ),
    ]);

    final ring = tester.widget<BudgetRing>(find.byType(BudgetRing));
    expect(ring.segments.single.isOver, isFalse);
  });

  testWidgets('tapping a legend row moves the middle to that category',
      (tester) async {
    await pump(
      tester,
      budget: [
        budgeted('Groceries', budget: 20000, spent: 7550, remaining: 12450),
        budgeted('Gas', budget: 30000, spent: 4200, remaining: 25800),
      ],
      totals: MonthTotals(budgeted: fromCore(50000), spent: fromCore(11750)),
    );

    await tester.tap(legendRow('Gas'));
    await tester.pumpAndSettle();

    expect(find.text(r'$42.00'), findsOneWidget);
    expect(find.text(r'of $300.00'), findsOneWidget);
    // The total is no longer the headline while a category is selected.
    expect(find.text(r'of $500.00'), findsNothing);

    await tester.tap(legendRow('Gas'));
    await tester.pumpAndSettle();
    expect(find.text(r'of $500.00'), findsOneWidget);
  });

  testWidgets('unbudgeted categories are counted, not listed', (tester) async {
    await pump(tester, budget: [
      budgeted('Groceries', budget: 60000, spent: 0, remaining: 60000),
      unbudgeted,
    ]);

    expect(find.text('Travel'), findsNothing);
    expect(find.textContaining('1 category has no budget'), findsOneWidget);
  });

  testWidgets('nothing budgeted says how to budget something', (tester) async {
    await pump(tester);

    expect(find.textContaining('Nothing is budgeted yet'), findsOneWidget);
    expect(find.textContaining('pacc budget set'), findsOneWidget);
  });

  testWidgets('shows the three most recent expenses', (tester) async {
    await pump(tester, recent: [
      expense("Macey's", 11840),
      expense('Chevron', 6105),
      expense('Costco', 14390),
      expense('Should not appear', 100),
    ]);

    expect(find.text("Macey's"), findsOneWidget);
    expect(find.text(r'$118.40'), findsOneWidget);
    expect(find.text('Should not appear'), findsNothing);
  });

  testWidgets('an expense with no store says so', (tester) async {
    await pump(tester, recent: [expense(null, 500)]);

    expect(find.text('No store'), findsOneWidget);
  });

  testWidgets('no expenses yet is said plainly', (tester) async {
    await pump(tester);

    expect(find.text('No expenses yet.'), findsOneWidget);
  });
}
