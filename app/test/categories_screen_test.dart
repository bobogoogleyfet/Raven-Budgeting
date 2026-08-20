import 'package:app/src/rust/api/categories.dart';
import 'package:app/src/screens/categories_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

CategoryDto category(String name, String group, {bool budgeted = true}) =>
    CategoryDto(id: 1, name: name, groupName: group, isBudgeted: budgeted);

Future<void> pump(WidgetTester tester, CategoryLoader load) =>
    tester.pumpWidget(MaterialApp(home: Scaffold(body: CategoriesScreen(load: load))));

void main() {
  testWidgets('groups rows under the group name, in the order given',
      (tester) async {
    await pump(tester, () async => [
          category('Rent', 'Bills'),
          category('Electric', 'Bills'),
          category('Groceries', 'Frequent'),
        ]);
    await tester.pumpAndSettle();

    expect(find.text('Bills'), findsOneWidget);
    expect(find.text('Frequent'), findsOneWidget);
    expect(find.text('Rent'), findsOneWidget);

    // One header per group, however many categories are under it.
    expect(
      tester.getTopLeft(find.text('Bills')).dy,
      lessThan(tester.getTopLeft(find.text('Groceries')).dy),
    );
  });

  testWidgets('marks which categories are budgeted', (tester) async {
    await pump(tester, () async => [
          category('Groceries', 'Frequent'),
          category('Uncategorized', 'System', budgeted: false),
        ]);
    await tester.pumpAndSettle();

    expect(find.text('Budgeted'), findsOneWidget);
  });

  testWidgets('shows the core message on failure, and retries', (tester) async {
    var attempts = 0;
    await pump(tester, () async {
      attempts++;
      if (attempts == 1) throw Exception('the database is not open yet');
      return [category('Groceries', 'Frequent')];
    });
    await tester.pumpAndSettle();

    expect(find.textContaining('the database is not open yet'), findsOneWidget);

    await tester.tap(find.text('Retry'));
    await tester.pumpAndSettle();

    expect(find.text('Groceries'), findsOneWidget);
  });
}
