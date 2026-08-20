import 'package:app/src/api.dart';
import 'package:app/src/money.dart';
import 'package:app/src/screens/inbox_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

CategoryDto category(int id, String name, String group) =>
    CategoryDto(id: id, name: name, groupName: group, isBudgeted: true);

TxnSummary quickAdd(int id, {String? merchant, int total = 4218}) => TxnSummary(
      id: id,
      txnDate: '2026-08-19',
      merchantName: merchant,
      total: fromCore(total),
      taxIsEstimated: false,
      needsReview: true,
      categories: const ['Uncategorized'],
    );

class Fake {
  Fake({this.rows = const [], this.fails = false});

  List<TxnSummary> rows;
  bool fails;
  final List<(int, int)> filed = [];
  int loads = 0;

  Future<List<TxnSummary>> load() async {
    loads++;
    return rows;
  }

  Future<void> file(int id, int categoryId) async {
    if (fails) throw Exception('\$2.40 is unassigned; add a split');
    filed.add((id, categoryId));
    rows = rows.where((r) => r.id != id).toList();
  }
}

Future<void> pump(WidgetTester tester, Fake fake) async {
  tester.view.physicalSize = const Size(900, 900);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);

  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: InboxScreen(
          load: fake.load,
          loadCategories: () async => [
            category(1, 'Groceries', 'Frequent'),
            category(2, 'Gas', 'Frequent'),
          ],
          file: fake.file,
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('an empty queue says where entries come from', (tester) async {
    await pump(tester, Fake());

    expect(
      find.text('Nothing waiting. Quick-added expenses show up here.'),
      findsOneWidget,
    );
  });

  testWidgets('a waiting entry shows its date, store and total',
      (tester) async {
    await pump(tester, Fake(rows: [quickAdd(1, merchant: 'Costco')]));

    expect(find.text('2026-08-19'), findsOneWidget);
    expect(find.text('Costco'), findsOneWidget);
    expect(find.text(r'$42.18'), findsOneWidget);
  });

  testWidgets('a quick-add with no store shows an em dash', (tester) async {
    await pump(tester, Fake(rows: [quickAdd(1)]));

    expect(find.text('—'), findsOneWidget);
  });

  testWidgets('filing is locked until a category is chosen', (tester) async {
    final fake = Fake(rows: [quickAdd(1)]);
    await pump(tester, fake);

    // There is no sensible default, and a receipt filed under the wrong
    // envelope is worse than one still waiting.
    expect(
      tester.widget<FilledButton>(find.widgetWithText(FilledButton, 'File it')).onPressed,
      isNull,
    );

    await tester.tap(find.byType(DropdownButtonFormField<int>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Frequent / Gas').last);
    await tester.pumpAndSettle();

    expect(
      tester.widget<FilledButton>(find.widgetWithText(FilledButton, 'File it')).onPressed,
      isNotNull,
    );
  });

  testWidgets('filing sends the chosen category and empties the row',
      (tester) async {
    final fake = Fake(rows: [quickAdd(1), quickAdd(2, merchant: 'Chevron')]);
    await pump(tester, fake);

    await tester.tap(find.byType(DropdownButtonFormField<int>).first);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Frequent / Groceries').last);
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, 'File it').first);
    await tester.pumpAndSettle();

    expect(fake.filed, [(1, 1)]);
    // The queue is re-read, so a filed row leaves and the count is honest.
    expect(fake.loads, greaterThan(1));
    expect(find.text('Chevron'), findsOneWidget);
  });

  testWidgets('filing the last one shows the empty state', (tester) async {
    final fake = Fake(rows: [quickAdd(1)]);
    await pump(tester, fake);

    await tester.tap(find.byType(DropdownButtonFormField<int>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Frequent / Groceries').last);
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, 'File it'));
    await tester.pumpAndSettle();

    expect(find.textContaining('Nothing waiting'), findsOneWidget);
  });

  testWidgets('a refused categorize says so on the row and keeps it',
      (tester) async {
    final fake = Fake(rows: [quickAdd(1)], fails: true);
    await pump(tester, fake);

    await tester.tap(find.byType(DropdownButtonFormField<int>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Frequent / Groceries').last);
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, 'File it'));
    await tester.pumpAndSettle();

    expect(find.textContaining('is unassigned'), findsOneWidget);
    expect(find.text(r'$42.18'), findsOneWidget);
  });

  testWidgets('a failure to load says so once, at the top', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: InboxScreen(
            load: () async => throw Exception('the database is not open yet'),
            loadCategories: () async => const [],
            file: (id, categoryId) async {},
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.textContaining('Could not load the inbox'), findsOneWidget);
  });
}
