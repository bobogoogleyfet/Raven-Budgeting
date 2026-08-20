import 'package:app/src/api.dart';
import 'package:app/src/money.dart';
import 'package:app/src/screens/set_budget_dialog.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

CategoryDto category(int id, String name) =>
    CategoryDto(id: id, name: name, groupName: 'Frequent', isBudgeted: true);

class Sent {
  Sent(this.categoryId, this.amount, this.period, this.month);

  final int categoryId;
  final Cents amount;
  final BudgetPeriod period;
  final String month;
}

Future<List<Sent>> pump(
  WidgetTester tester, {
  Superseded result =
      const Superseded(restated: false, closedAfter: null),
  Object? error,
}) async {
  final sent = <Sent>[];
  tester.view.physicalSize = const Size(900, 1000);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);

  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: SetBudgetDialog(
          month: '2026-08',
          categories: [category(1, 'Groceries'), category(2, 'Gas')],
          save: ({
            required int categoryId,
            required Cents amount,
            required BudgetPeriod period,
            required String month,
            bool rollover = false,
          }) async {
            if (error != null) throw error;
            sent.add(Sent(categoryId, amount, period, month));
            return result;
          },
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
  return sent;
}

Future<void> choose(WidgetTester tester, String name) async {
  await tester.tap(find.byType(DropdownButtonFormField<int>));
  await tester.pumpAndSettle();
  await tester.tap(find.text('Frequent / $name').last);
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('a budget goes as cents, for the month it starts from',
      (tester) async {
    final sent = await pump(tester);
    await choose(tester, 'Groceries');
    await tester.enterText(find.byType(TextField), '250.00');
    await tester.tap(find.widgetWithText(FilledButton, 'Save budget'));
    await tester.pumpAndSettle();

    expect(sent.single.categoryId, 1);
    expect(sent.single.amount.value, 25000);
    expect(sent.single.period, BudgetPeriod.monthly);
    expect(sent.single.month, '2026-08');
  });

  testWidgets('a yearly budget says it is the whole year', (tester) async {
    await pump(tester);
    await tester.tap(find.text('Once a year'));
    await tester.pumpAndSettle();

    // A $1,200 bill is about $100 a month, and the core does that division.
    expect(find.textContaining('saved for across twelve'), findsOneWidget);
  });

  testWidgets('an unparseable amount is refused, not read as zero',
      (tester) async {
    final sent = await pump(tester);
    await choose(tester, 'Groceries');
    await tester.enterText(find.byType(TextField), 'lots');
    await tester.tap(find.widgetWithText(FilledButton, 'Save budget'));
    await tester.pumpAndSettle();

    // A $0.00 budget is a claim that this category should have nothing.
    expect(sent, isEmpty);
    expect(find.textContaining('Enter an amount'), findsOneWidget);
  });

  testWidgets('no category is refused', (tester) async {
    final sent = await pump(tester);
    await tester.enterText(find.byType(TextField), '250');
    await tester.tap(find.widgetWithText(FilledButton, 'Save budget'));
    await tester.pumpAndSettle();

    expect(sent, isEmpty);
    expect(find.textContaining('Pick a category'), findsOneWidget);
  });

  testWidgets('a refused save says why and stays open', (tester) async {
    final sent = await pump(tester, error: Exception('there is already one covering 2026-01..2026-06'));
    await choose(tester, 'Groceries');
    await tester.enterText(find.byType(TextField), '250');
    await tester.tap(find.widgetWithText(FilledButton, 'Save budget'));
    await tester.pumpAndSettle();

    expect(sent, isEmpty);
    expect(find.textContaining('already one covering'), findsOneWidget);
    expect(find.text('Set a budget'), findsOneWidget);
  });

  test('what it did is said in words', () {
    expect(
      budgetOutcome(const Superseded(restated: true, closedAfter: null), 'Gas'),
      'Updated the budget for Gas.',
    );
    expect(
      budgetOutcome(const Superseded(restated: false, closedAfter: null), 'Gas'),
      'Budgeted Gas.',
    );
    // The month the old line now ends in is the part a user cannot see.
    expect(
      budgetOutcome(
        const Superseded(restated: false, closedAfter: '2026-07'),
        'Gas',
      ),
      'Budgeted Gas. The previous budget now ends July 2026.',
    );
  });
}
